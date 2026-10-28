//! Coletor de lixo (`ARQUITETURA.md`, seção 13).
//!
//! - **Menor**: copia os vivos do berçário para o fim da geração velha (Cheney). Raízes: as do
//!   motor, os campos lembrados pela barreira de escrita e as tabelas de Map/Set (que moram no
//!   Rust, sem barreira).
//! - **Maior**: logo depois de uma menor (berçário vazio), marca a velha e compacta deslizando.
//!   Toda palavra de 8 bytes de um vivo fica marcada no mapa de bits; o endereço novo é a soma
//!   dos vivos antes do bloco de 64 palavras mais os vivos antes dele no bloco. Sem tabela de
//!   encaminhamento por objeto.
//! - WeakMap/WeakSet são efêmeros na maior: o valor só vive se a chave vive, e a entrada com
//!   chave morta sai. Tabelas de Map/Set mortos são liberadas.
//!
//! Só roda em ponto seguro ([`Motor::pode_coletar`]): nenhum `Valor` solto no Rust além das
//! raízes. Os callbacks de `forEach`/`map`/`filter`/`some`/`every` coletam
//! ([`Motor::chamar_coletavel`]).
//!
//! shortcut: o resto do JS chamado de dentro de nativa (eval, `new Function`, `importScripts`,
//! getters, `reduce`, `sort`, async antes do 1º await) não coleta até a nativa voltar; upgrade:
//! passar a nativa para `chamar_coletavel`, com o que ela usa depois da chamada em `raizes`.
//!
//! shortcut: formas nunca morrem e o protótipo delas é raiz; o mesmo vale para os quadros de
//! geradores largados no meio (`suspensos`) e para o alvo de WeakRef (propriedade comum); a
//! gaiola não devolve memória ao sistema depois de encolher. Upgrade: coletar formas sem
//! objetos, ligar o suspenso ao objeto do gerador e encurtar `Gaiola::mem` após a maior.

use crate::builtins::colecoes::{ChaveTabela, Tabela};
use crate::formas::{Classe, FormaId, Formas, Tipo};
use crate::funcoes::ConstRt;
use crate::gaiola::Gaiola;
use crate::motor::Motor;
use crate::valor::Valor;

/// Palavra 0 de objeto do berçário já copiado; a palavra 1 guarda o endereço novo.
const ENCAMINHADO: u32 = u32::MAX;
/// Abaixo disto a geração velha nunca pede coleta maior.
const MAIOR_MIN: u64 = 32 << 20;

/// Crescimento da velha que pede coleta sozinho (sem o berçário encher): o dobro do limite da
/// coleta maior, para só pesar quando objetos grandes (buffers, textos) se acumulam.
pub(crate) fn gatilho_velho(vivo: u64) -> u64 {
    2 * MAIOR_MIN.max(vivo)
}
/// Bit do `with` no tamanho do contexto (ver `interpretador`).
const CTX_COM: u32 = 1 << 31;

/// Onde estão as referências de um objeto (deslocamentos desde o início dele).
#[derive(Clone, Copy)]
struct Refs {
    faixas: [(u32, u32); 2],
    internos: u32,
    mascara: u32,
    classe: Option<Classe>,
}

impl Refs {
    const NADA: Refs = Refs { faixas: [(0, 0); 2], internos: 0, mascara: 0, classe: None };

    fn faixa(a: u32, b: u32) -> Refs {
        Refs { faixas: [(a, b), (0, 0)], ..Refs::NADA }
    }

    /// Chama `f` com o endereço de cada campo que guarda valor.
    #[inline]
    fn cada(&self, d: u32, mut f: impl FnMut(u32)) {
        for &(a, b) in &self.faixas {
            let mut o = a;
            while o < b {
                f(d + o);
                o += 4;
            }
        }
        for i in 0..self.internos {
            if self.mascara & (1 << i) != 0 {
                f(d + 12 + 4 * i);
            }
        }
    }
}

/// Tamanho (arredondado a 8) e referências do objeto em `d` (layouts em `formas::Tipo`).
fn ler(g: &Gaiola, formas: &Formas, d: u32) -> (u32, Refs) {
    let f = formas.get(FormaId(g.u32_em(d)));
    let len = || g.u32_em(d + 4);
    let (b, r) = match f.tipo {
        Tipo::Fixo => (8, Refs::NADA),
        Tipo::Numero => (16, Refs::NADA),
        Tipo::Latin1 => (16 + len(), Refs::NADA),
        Tipo::Utf16 => (16 + 2 * len(), Refs::NADA),
        Tipo::Corda => (24, Refs::faixa(16, 24)),
        Tipo::Lista => {
            let n = 8 + 4 * len();
            (n, Refs::faixa(8, n))
        }
        Tipo::ListaDouble => (8 + 8 * len(), Refs::NADA),
        Tipo::Bytes => (8 + len(), Refs::NADA),
        Tipo::Simbolo => (12, Refs::faixa(8, 12)),
        Tipo::BigInt => (8 + 4 * (len() & !(1 << 31)), Refs::NADA),
        Tipo::Acessor => (12, Refs::faixa(4, 12)),
        Tipo::Contexto => {
            let n = 12 + 4 * (len() & !CTX_COM);
            (n, Refs::faixa(8, n))
        }
        Tipo::Objeto => {
            let (ni, mascara) = f.classe.internos();
            let ini = 12 + 4 * ni;
            let fim = ini + 4 * f.inline;
            (fim, Refs { faixas: [(4, 12), (ini, fim)], internos: ni, mascara, classe: Some(f.classe) })
        }
    };
    ((b + 7) & !7, r)
}

/// Visita cada raiz do motor (menos os protótipos das formas, tratados à parte).
fn raizes(m: &mut Motor, extra: &mut [Valor], f: &mut dyn FnMut(&mut Gaiola, &Formas, &mut Valor)) {
    let Motor {
        g,
        formas,
        nomes,
        simbolos,
        pilha,
        quadros,
        protos,
        poli,
        mega,
        intr,
        lexico_valores,
        chamado,
        suspensos,
        microtarefas,
        promessas_async,
        templates,
        raizes,
        registro_simbolos,
        rejeicoes,
        modulos,
        ..
    } = m;
    let formas: &Formas = formas;
    let mut v = |x: &mut Valor| f(g, formas, x);
    extra.iter_mut().for_each(&mut v);
    nomes.valores.iter_mut().for_each(&mut v);
    simbolos.iter_mut().for_each(&mut v);
    pilha.iter_mut().for_each(&mut v);
    lexico_valores.iter_mut().for_each(&mut v);
    templates.iter_mut().for_each(&mut v);
    raizes.iter_mut().for_each(&mut v);
    rejeicoes.iter_mut().for_each(&mut v);
    promessas_async.values_mut().for_each(&mut v);
    registro_simbolos.values_mut().for_each(&mut v);
    v(chamado);
    for md in modulos.iter_mut() {
        md.valores_mut().for_each(&mut v);
    }
    for q in quadros.iter_mut() {
        [&mut q.this, &mut q.funcao, &mut q.new_target, &mut q.contexto].into_iter().for_each(&mut v);
    }
    for s in suspensos.iter_mut().flatten() {
        let q = &mut s.quadro;
        [&mut q.this, &mut q.funcao, &mut q.new_target, &mut q.contexto, &mut s.promessa].into_iter().for_each(&mut v);
        s.valores.iter_mut().for_each(&mut v);
    }
    for t in microtarefas.iter_mut() {
        t.valores_mut().into_iter().for_each(&mut v);
    }
    for p in protos.iter_mut() {
        v(&mut p.nome);
        for c in p.consts.iter_mut() {
            match c {
                ConstRt::Valor(x) | ConstRt::Classe { nome: x, .. } => v(x),
                _ => {}
            }
        }
        for c in p.caches.iter_mut() {
            v(&mut c.dono);
        }
    }
    poli.iter_mut().flatten().for_each(|c| v(&mut c.dono));
    mega.values_mut().for_each(|c| v(&mut c.dono));
    let i = intr;
    [
        &mut i.global, &mut i.objeto_proto, &mut i.funcao_proto, &mut i.array_proto, &mut i.texto_proto,
        &mut i.numero_proto, &mut i.booleano_proto, &mut i.simbolo_proto, &mut i.bigint_proto,
        &mut i.iterador_proto, &mut i.iterador_helper_proto, &mut i.wrap_iterador_proto,
        &mut i.array_buffer_proto, &mut i.shared_array_buffer_proto, &mut i.atomics_esperas, &mut i.array_iterador_proto,
        &mut i.gerador_funcao_proto, &mut i.gerador_proto, &mut i.promessa_proto, &mut i.regex_proto,
        &mut i.async_funcao_proto, &mut i.async_gerador_funcao_proto, &mut i.async_gerador_proto,
        &mut i.async_iterador_proto, &mut i.async_de_sync_proto, &mut i.promessa_ctor, &mut i.promessa_js,
        &mut i.regex_exec, &mut i.eval, &mut i.array_valores, &mut i.array_iterador_next,
        &mut i.funcao_has_instance,
    ]
    .into_iter()
    .for_each(&mut v);
    i.erro_protos.iter_mut().for_each(&mut v);
    i.tipados_protos.iter_mut().for_each(&mut v);
    i.tipados_ctors.iter_mut().for_each(&mut v);
}

/// Passa `mudar` nos protótipos das formas e refaz o índice de raízes delas.
fn protos_das_formas(m: &mut Motor, mut mudar: impl FnMut(&Gaiola, &Formas, Valor) -> Valor) {
    let antes: Vec<Valor> = m.formas.protos_mut().map(|p| *p).collect();
    let depois: Vec<Valor> = antes.iter().map(|&p| mudar(&m.g, &m.formas, p)).collect();
    if antes != depois {
        m.formas.protos_mut().zip(depois).for_each(|(p, n)| *p = n);
        m.formas.refazer_raizes();
    }
}

// ---------------------------------------------------------------- menor

/// Copia `v` do berçário para a velha (uma vez só) e devolve o endereço novo.
fn promover(g: &mut Gaiola, formas: &Formas, v: Valor) -> Valor {
    if !v.e_ref() || !g.no_bercario(v.deslocamento()) {
        return v;
    }
    let d = v.deslocamento();
    if g.u32_em(d) == ENCAMINHADO {
        return Valor::de_deslocamento(g.u32_em(d + 4));
    }
    let (n, _) = ler(g, formas, d);
    let novo = g.alocar_velho(n);
    g.copiar(d, novo, n);
    g.por_u32(d, ENCAMINHADO);
    g.por_u32(d + 4, novo);
    Valor::de_deslocamento(novo)
}

/// Itens de cada tabela passam por `mudar`; chave de objeto que mudou de lugar troca no índice.
fn tabelas(g: &mut Gaiola, formas: &Formas, tabs: &mut [Tabela], mut mudar: impl FnMut(&mut Gaiola, &Formas, Valor) -> Valor) {
    for t in tabs {
        let mut trocas = Vec::new();
        for (k, v) in t.itens.iter_mut().flatten() {
            let nk = mudar(g, formas, *k);
            if nk != *k {
                trocas.push((k.0, nk.0));
                *k = nk;
            }
            *v = mudar(g, formas, *v);
        }
        for (a, b) in trocas {
            if let Some(p) = t.indice.remove(&ChaveTabela::Ref(a)) {
                t.indice.insert(ChaveTabela::Ref(b), p);
            }
        }
    }
}

impl Motor {
    /// Coleta se o berçário encheu e nada do Rust segura valores (o embutidor chama entre
    /// tarefas; o intérprete chama nos pontos seguros dele).
    pub fn coletar_se_preciso(&mut self) {
        if self.pode_coletar() && self.g.coleta_pendente {
            self.coletar(&mut []);
        }
    }

    /// Todo nível de nativa aberto é seguro: nenhum `Valor` solto no Rust.
    #[inline]
    pub fn pode_coletar(&self) -> bool {
        self.aninhamento == self.seguro
    }

    /// Coleta agora. `extra`: valores do chamador que também são raízes (e são atualizados).
    pub fn coletar(&mut self, extra: &mut [Valor]) {
        // Os endereços mudam: o erro guardado para refazer a pilha não vale mais.
        self.erro_interno = (Valor::UNDEFINED, 0);
        debug_assert!(self.pode_coletar(), "coleta fora de ponto seguro");
        self.coleta_menor(extra);
        self.coletas[0] += 1;
        let limite = MAIOR_MIN.max(self.vivo_apos_maior);
        if self.g.velho_desde_coleta > limite || (self.estresse && self.coletas[0].is_multiple_of(8)) {
            self.coleta_maior(extra);
            self.coletas[1] += 1;
        }
        self.g.gatilho_velho = gatilho_velho(self.vivo_apos_maior);
        self.g.coleta_pendente = self.estresse;
    }

    /// Menor e maior agora, sem olhar os limites (a gaiola passou do teto).
    pub(crate) fn coletar_tudo(&mut self, extra: &mut [Valor]) {
        self.erro_interno = (Valor::UNDEFINED, 0);
        self.coleta_menor(extra);
        self.coleta_maior(extra);
        self.coletas[0] += 1;
        self.coletas[1] += 1;
        self.g.gatilho_velho = gatilho_velho(self.vivo_apos_maior);
        self.g.coleta_pendente = self.estresse;
    }

    fn coleta_menor(&mut self, extra: &mut [Valor]) {
        let inicio = self.g.topo_velho();
        raizes(self, extra, &mut |g, formas, v| *v = promover(g, formas, *v));
        let lembrados = std::mem::take(&mut self.g.lembrados);
        for s in lembrados {
            let v = self.g.valor_em(s);
            let n = promover(&mut self.g, &self.formas, v);
            if n != v {
                self.g.por_u32(s, n.0);
            }
        }
        // Os protótipos das formas também são raízes (mudar precisa de &mut Gaiola).
        let antes: Vec<Valor> = self.formas.protos_mut().map(|p| *p).collect();
        let depois: Vec<Valor> = antes.iter().map(|&p| promover(&mut self.g, &self.formas, p)).collect();
        if antes != depois {
            self.formas.protos_mut().zip(depois).for_each(|(p, n)| *p = n);
            self.formas.refazer_raizes();
        }
        tabelas(&mut self.g, &self.formas, &mut self.tabelas, promover);
        // Cheney: o que foi copiado ainda aponta para o berçário; varre até alcançar o topo.
        let mut s = inicio;
        while s < self.g.topo_velho() {
            let (n, r) = ler(&self.g, &self.formas, s);
            let (g, formas) = (&mut self.g, &self.formas);
            r.cada(s, |c| {
                let v = g.valor_em(c);
                let p = promover(g, formas, v);
                if p != v {
                    g.por_u32(c, p.0);
                }
            });
            s += n;
        }
        self.g.esvaziar_bercario();
    }

    // ---------------------------------------------------------------- maior

    fn coleta_maior(&mut self, extra: &mut [Valor]) {
        debug_assert!(self.g.lembrados.is_empty() && self.g.topo_bercario() == self.g.bercario_inicio);
        let mut mc = Marcacao::nova(&self.g, self.tabelas.len());
        raizes(self, extra, &mut |g, formas, v| mc.marcar(g, formas, *v));
        let ps: Vec<Valor> = self.formas.protos_mut().map(|p| *p).collect();
        for p in ps {
            mc.marcar(&self.g, &self.formas, p);
        }
        mc.drenar(self);
        // Efêmeros: valor de WeakMap vive se a chave vive; repete até parar de mudar.
        loop {
            let mut mudou = false;
            let mut i = 0;
            while i < mc.fracas.len() {
                let t = mc.fracas[i];
                for &(k, v) in self.tabelas[t].itens.iter().flatten() {
                    if mc.vivo(k) && !mc.vivo(v) {
                        mc.marcar(&self.g, &self.formas, v);
                        mudou = true;
                    }
                }
                i += 1;
            }
            mc.drenar(self);
            if !mudou {
                break;
            }
        }
        for &t in &mc.fracas {
            let tab = &mut self.tabelas[t];
            for item in tab.itens.iter_mut() {
                if let Some((k, _)) = *item {
                    if !mc.vivo(k) {
                        tab.indice.remove(&ChaveTabela::Ref(k.0));
                        *item = None;
                        tab.vivos -= 1;
                    }
                }
            }
        }
        for (t, &viva) in mc.tab_viva.iter().enumerate() {
            if !viva {
                self.tabelas[t] = Tabela::nova();
            }
        }

        // Endereços novos.
        let fw = Encaminhamento::novo(mc.base, mc.bits);
        raizes(self, extra, &mut |_, _, v| *v = fw.valor(*v));
        protos_das_formas(self, |_, _, p| fw.valor(p));
        let vivas: Vec<bool> = mc.tab_viva;
        for (t, tab) in self.tabelas.iter_mut().enumerate() {
            if !vivas[t] {
                continue;
            }
            for (k, v) in tab.itens.iter_mut().flatten() {
                *k = fw.valor(*k);
                *v = fw.valor(*v);
            }
            if tab.indice.keys().any(|c| matches!(c, ChaveTabela::Ref(_))) {
                tab.indice = tab
                    .indice
                    .drain()
                    .map(|(c, p)| match c {
                        ChaveTabela::Ref(x) => (ChaveTabela::Ref(fw.valor(Valor(x)).0), p),
                        c => (c, p),
                    })
                    .collect();
            }
        }
        // Campos dos vivos (ainda no lugar velho), depois o deslize.
        let topo = self.g.topo_velho();
        let mut d = fw.base;
        while let Some(o) = fw.proximo(d, topo) {
            let (n, r) = ler(&self.g, &self.formas, o);
            let g = &mut self.g;
            r.cada(o, |c| {
                let v = g.valor_em(c);
                let nv = fw.valor(v);
                if nv != v {
                    g.por_u32(c, nv.0);
                }
            });
            d = o + n;
        }
        let mut d = fw.base;
        while let Some(o) = fw.proximo(d, topo) {
            let (n, _) = ler(&self.g, &self.formas, o);
            let para = fw.endereco(o);
            if para != o {
                self.g.copiar(o, para, n);
            }
            d = o + n;
        }
        self.g.definir_topo_velho(fw.base + fw.total);
        self.g.velho_desde_coleta = 0;
        self.vivo_apos_maior = u64::from(fw.total);
    }
}

/// Estado da marcação da coleta maior.
struct Marcacao {
    base: u32,
    topo: u32,
    /// Um bit por palavra de 8 bytes da velha.
    bits: Vec<u64>,
    pilha: Vec<u32>,
    tab_viva: Vec<bool>,
    fracas: Vec<usize>,
}

impl Marcacao {
    fn nova(g: &Gaiola, n_tabelas: usize) -> Self {
        let (base, topo) = (g.velho_inicio, g.topo_velho());
        let palavras = ((topo - base) / 8) as usize;
        Self { base, topo, bits: vec![0; palavras.div_ceil(64) + 1], pilha: Vec::new(), tab_viva: vec![false; n_tabelas], fracas: Vec::new() }
    }

    #[inline]
    fn marcado(&self, d: u32) -> bool {
        let w = ((d - self.base) / 8) as usize;
        self.bits[w / 64] & (1 << (w % 64)) != 0
    }

    /// Fora da velha (fixos, permanentes) conta como vivo.
    fn vivo(&self, v: Valor) -> bool {
        !v.e_ref() || v.deslocamento() < self.base || self.marcado(v.deslocamento())
    }

    fn marcar(&mut self, g: &Gaiola, formas: &Formas, v: Valor) {
        if !v.e_ref() {
            return;
        }
        let d = v.deslocamento();
        if d < self.base || self.marcado(d) {
            return;
        }
        debug_assert!(d < self.topo, "referência além do topo da gaiola: {d}");
        let (n, _) = ler(g, formas, d);
        let (mut w, fim) = (((d - self.base) / 8) as usize, ((d + n - self.base) / 8) as usize);
        while w < fim {
            let (b, i) = (w / 64, w % 64);
            let k = (64 - i).min(fim - w);
            let mascara = if k == 64 { u64::MAX } else { ((1u64 << k) - 1) << i };
            self.bits[b] |= mascara;
            w += k;
        }
        self.pilha.push(d);
    }

    fn drenar(&mut self, m: &Motor) {
        while let Some(d) = self.pilha.pop() {
            let (_, r) = ler(&m.g, &m.formas, d);
            r.cada(d, |c| self.marcar(&m.g, &m.formas, m.g.valor_em(c)));
            let Some(classe) = r.classe else { continue };
            let fraca = match classe {
                Classe::Mapa | Classe::Conjunto => false,
                Classe::MapaFraco | Classe::ConjuntoFraco => true,
                _ => continue,
            };
            let t = m.g.u32_em(d + 16) as usize;
            if t >= self.tab_viva.len() || self.tab_viva[t] {
                continue;
            }
            self.tab_viva[t] = true;
            if fraca {
                self.fracas.push(t);
            } else {
                for &(k, v) in m.tabelas[t].itens.iter().flatten() {
                    self.marcar(&m.g, &m.formas, k);
                    self.marcar(&m.g, &m.formas, v);
                }
            }
        }
    }
}

/// Endereço novo de cada vivo da velha, pelo mapa de bits.
struct Encaminhamento {
    base: u32,
    bits: Vec<u64>,
    /// Bytes vivos antes de cada bloco de 64 palavras.
    antes: Vec<u32>,
    total: u32,
}

impl Encaminhamento {
    fn novo(base: u32, bits: Vec<u64>) -> Self {
        let mut antes = Vec::with_capacity(bits.len());
        let mut total = 0u32;
        for w in &bits {
            antes.push(total);
            total += 8 * w.count_ones();
        }
        Self { base, bits, antes, total }
    }

    #[inline]
    fn endereco(&self, d: u32) -> u32 {
        let w = (d - self.base) / 8;
        let (b, i) = ((w / 64) as usize, w % 64);
        self.base + self.antes[b] + 8 * (self.bits[b] & ((1u64 << i) - 1)).count_ones()
    }

    #[inline]
    fn valor(&self, v: Valor) -> Valor {
        if v.e_ref() && v.deslocamento() >= self.base {
            Valor::de_deslocamento(self.endereco(v.deslocamento()))
        } else {
            v
        }
    }

    /// Primeiro vivo em `[d, topo)`.
    fn proximo(&self, d: u32, topo: u32) -> Option<u32> {
        let mut w = ((d - self.base) / 8) as usize;
        let fim = ((topo - self.base) / 8) as usize;
        while w < fim {
            let (b, i) = (w / 64, w % 64);
            let resto = self.bits[b] >> i;
            if resto != 0 {
                let achou = w + resto.trailing_zeros() as usize;
                return (achou < fim).then(|| self.base + 8 * achou as u32);
            }
            w = (b + 1) * 64;
        }
        None
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn rodar(m: &mut Motor, fonte: &str) -> String {
        let v = m.avaliar(fonte).unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        m.rodar_microtarefas();
        m.para_texto_rust(v).unwrap()
    }

    fn estressado() -> Motor {
        let mut m = Motor::novo();
        m.estresse = true;
        m.g.coleta_pendente = true;
        m
    }

    #[test]
    fn objetos_sobrevivem_e_lixo_some() {
        let mut m = estressado();
        let r = rodar(
            &mut m,
            "let lista = null;
             for (let i = 0; i < 20000; i++) { lista = { i, prox: lista, t: 'n' + i }; let lixo = [i, {i}]; }
             let s = 0, n = 0;
             for (let p = lista; p; p = p.prox) { s += p.i; n++; if (p.t !== 'n' + p.i) throw new Error('texto') }
             s + ':' + n",
        );
        assert_eq!(r, format!("{}:20000", (0..20000u64).sum::<u64>()));
        assert!(m.coletas[0] > 100 && m.coletas[1] > 10, "{:?}", m.coletas);
    }

    /// Coleta dentro dos callbacks de forEach/map/filter/some/every: o array, o callback, o
    /// resultado em montagem e o item continuam certos depois de mudarem de endereço.
    #[test]
    fn coleta_dentro_de_callback() {
        let mut m = estressado();
        let r = rodar(&mut m, "[1].forEach(function () { for (let i = 0; i < 1000; i++) { let lixo = [i, {i}]; } }); 'ok'");
        assert_eq!(r, "ok");
        assert!(m.coletas[0] > 500, "o callback não coletou: {:?}", m.coletas);
        let r = rodar(
            &mut m,
            "const d = Array.from({length: 300}, (_, i) => ({id: i, nome: 'item' + i}));
             let soma = 0;
             d.forEach((x, i, arr) => { const l = {a: [x.id]}; soma += l.a[0]; if (arr !== d) throw new Error('arr'); });
             const m = d.map(x => ({dobro: x.id * 2, txt: x.nome + '!'}));
             const f = d.filter((x, i) => [i, {i}][1].i % 3 === 0);
             const s = d.some(x => ({x}).x.id === 299), e = d.every(x => [x][0].nome === 'item' + x.id);
             const dentro = [1, 2].map(k => d.map(x => ({v: x.id + k})).filter(o => o.v % 100 === 0).length);
             const red = d.reduce((ac, x) => ({t: ac.t + [x.id][0]}), {t: 0}).t;
             const mapa = new Map(d.map(x => [x.id, {x}])), conj = new Set(d);
             let pm = 0, pc = 0;
             mapa.forEach((v, k, mm) => { pm += v.x.id + ({k}).k; if (mm !== mapa) throw new Error('mapa'); });
             conj.forEach(x => { pc += [x][0].id; });
             const via = (function (a, b) { return [this.k, a.v, b.v].join('/'); }).call({k: 'c'}, {v: 1}, {v: 2})
               + (function (a) { return [{a}][0].a; }).apply(null, ['ap']) + Reflect.apply(x => ({x}).x, null, ['rf']);
             [soma, m[299].dobro, m[123].txt, f.length, f[99].id, s, e, dentro, red, pm, pc, via].join()",
        );
        assert_eq!(r, "44850,598,item123!,100,297,true,true,3,3,44850,89700,44850,c/1/2aprf");
    }

    #[test]
    fn closures_mapas_geradores_e_async() {
        let mut m = estressado();
        let r = rodar(
            &mut m,
            "const mapa = new Map(), conj = new Set();
             const fs = [];
             for (let i = 0; i < 3000; i++) {
               const k = {i};
               mapa.set(k, 'v' + i); conj.add(k);
               fs.push(() => k.i * 2);
             }
             let soma = 0;
             for (const [k, v] of mapa) { if (v !== 'v' + k.i || !conj.has(k)) throw new Error('mapa'); soma += k.i }
             for (const f of fs) soma += f();
             function* g() { let a = []; for (let i = 0; i < 2000; i++) { a.push({i}); yield i } }
             for (const x of g()) soma += x;
             globalThis.fim = 0;
             (async () => { for (let i = 0; i < 500; i++) { await null; fim += [i, {i}][1].i } })();
             soma",
        );
        let esperado: u64 = (0..3000u64).sum::<u64>() * 3 + (0..2000u64).sum::<u64>();
        assert_eq!(r, esperado.to_string());
        assert_eq!(rodar(&mut m, "fim"), (0..500u64).sum::<u64>().to_string());
    }

    #[test]
    fn weakmap_solta_a_chave_morta() {
        let mut m = Motor::novo();
        rodar(
            &mut m,
            "globalThis.wm = new WeakMap(); globalThis.fica = {};
             wm.set(fica, {vivo: 1});
             for (let i = 0; i < 1000; i++) wm.set({i}, {i});",
        );
        m.coletar(&mut []);
        m.coleta_maior(&mut []);
        let wm = m.avaliar("wm").unwrap();
        let t = m.interno(wm, 1) as usize;
        assert_eq!(m.tabelas[t].vivos, 1);
        assert_eq!(rodar(&mut m, "wm.get(fica).vivo"), "1");
    }

    #[test]
    fn memoria_nao_cresce_com_lixo() {
        let mut m = Motor::novo();
        rodar(&mut m, "let x; for (let i = 0; i < 3000000; i++) { x = {a: i, b: [i, i + 1], c: 'k' + i } } x.a");
        assert!(m.g.tamanho_reservado() < 64 << 20, "gaiola com {} MB", m.g.tamanho_reservado() >> 20);
        assert!(m.coletas[0] > 0);
    }
}
