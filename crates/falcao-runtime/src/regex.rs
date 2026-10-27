//! Motor de expressões regulares do JavaScript (ECMAScript 2023) do Falcão.
//!
//! Rust puro, só com a biblioteca padrão. Trabalha sobre texto UTF-16
//! (`&[u16]`), como as strings do JavaScript. O casamento é backtracking
//! com continuações. Quantificadores sobre átomos simples (caractere, classe,
//! ponto) são laços iterativos com recuo, sem recursão por volta.
//!
//! Limites conhecidos (também marcados com `shortcut:` no código):
//! - quantificador sobre grupo usa recursão: passado o limite de profundidade
//!   o casamento devolve None;
//! - lookbehind tenta todos os inícios para trás (quadrático);
//! - modo `v`: conjuntos (`--`, `&&`, aninhados, `\q{..}`) sim; propriedades de
//!   strings (`\p{RGI_Emoji}` etc.) não (sem dados de emoji-sequences);
//! - tabelas Unicode (`\p{..}`) são as oficiais de `unicode_tabelas.rs`, geradas do UCD.

/// Flags de uma expressão regular, lidas com `Flags::de_texto`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub global: bool,
    pub ignorar_caixa: bool,
    pub multilinha: bool,
    pub ponto_tudo: bool,
    pub unicode: bool,
    pub aderente: bool,
    pub indices: bool,
    pub v: bool,
}

/// Erro antecipado: cada literal `/…/f` do script tem de compilar (antes de rodar qualquer código).
pub fn validar_literais(script: &falcao_bytecode::Script) -> Result<(), String> {
    for fb in &script.funcoes {
        for c in &fb.constantes {
            if let falcao_bytecode::Constante::Regex { corpo, flags } = c {
                let f = Flags::de_texto(&script.textos[*flags as usize])?;
                Programa::compilar(&script.textos[*corpo as usize], &f)?;
            }
        }
    }
    Ok(())
}

impl Flags {
    /// Lê as flags como texto ("dgimsuyv"). Letra desconhecida ou repetida é erro.
    pub fn de_texto(f: &[u16]) -> Result<Flags, String> {
        let mut x = Flags::default();
        for &u in f {
            let c = char::from_u32(u as u32).unwrap_or('?');
            let campo: &mut bool = match c {
                'd' => &mut x.indices,
                'g' => &mut x.global,
                'i' => &mut x.ignorar_caixa,
                'm' => &mut x.multilinha,
                's' => &mut x.ponto_tudo,
                'u' => &mut x.unicode,
                'y' => &mut x.aderente,
                'v' => &mut x.v,
                _ => return Err(format!("flag desconhecida: {}", c)),
            };
            if *campo {
                return Err(format!("flag repetida: {}", c));
            }
            *campo = true;
        }
        if x.unicode && x.v {
            return Err("as flags u e v não podem ser usadas juntas".to_string());
        }
        Ok(x)
    }
}

/// Passos de casamento permitidos antes de desistir (devolve None).
const LIMITE_PASSOS: u64 = 50_000_000;
/// Passos somados de todos os inícios de uma busca (~2 s): por início, `/(?<=a+)b/` em 200 mil
/// `a` gastaria até LIMITE_PASSOS em cada um.
const LIMITE_BUSCA: u64 = 1_000_000_000;
/// shortcut: 1500 níveis de recursão para grupos repetidos; sobe se a pilha permitir.
const LIMITE_PROFUNDIDADE: usize = 1500;

/// Nó da árvore de expressão.
enum No {
    Char(u32),
    Classe(Classe),
    Qualquer,
    Inicio,
    Fim,
    /// `true` = `\b`, `false` = `\B`.
    Fronteira(bool),
    /// Grupo; índice 0 = não captura.
    Grupo(Box<No>, usize),
    Alt(Vec<No>),
    Seq(Vec<No>),
    Rep(Box<No>, Quant),
    /// Retro-referência ao grupo de índice dado.
    Ref(usize),
    /// Lookaround: (interno, positivo, atrás).
    Olhar(Box<No>, bool, bool),
}

/// Quantificador. `a..b` é a faixa de grupos de captura dentro do átomo.
#[derive(Clone, Copy)]
struct Quant {
    min: usize,
    max: Option<usize>,
    guloso: bool,
    a: usize,
    b: usize,
}

struct Classe {
    itens: Vec<Item>,
    negada: bool,
    /// Resposta pronta para os caracteres < 256 (já com `i` e a negação), feita em `preparar`.
    mapa: [u64; 4],
}

#[derive(Clone, Copy)]
enum Item {
    Faixa(u32, u32),
    /// Predicado (\d, \w, \s, \p{..}) e se está negado.
    Pred(Pred, bool),
}

#[derive(Clone, Copy)]
enum Pred {
    Dig,
    Pal,
    Esp,
    /// `\p{..}`: faixas ordenadas (tabelas de `unicode_tabelas`).
    Tab(&'static [(u32, u32)]),
}

/// Estado de uma tentativa de casamento.
struct Estado {
    caps: Vec<Option<(usize, usize)>>,
    passos: u64,
    prof: usize,
    abortou: bool,
}

/// Conta um passo. Devolve false se o limite foi estourado.
fn conta(e: &mut Estado) -> bool {
    e.passos += 1;
    if e.passos > LIMITE_PASSOS {
        e.abortou = true;
    }
    !e.abortou
}

/// Expressão regular compilada.
pub struct Programa {
    raiz: No,
    grupos: usize,
    nomes: Vec<(Vec<u16>, usize)>,
    opc: Flags,
    prefixo: Option<u16>,
    /// Unidades que podem abrir um casamento (só < 256): `procurar` pula as outras.
    primeiros: Option<[u64; 4]>,
}

impl Programa {
    /// Compila o padrão (texto UTF-16). Err(mensagem) = SyntaxError.
    pub fn compilar(padrao: &[u16], flags: &Flags) -> Result<Programa, String> {
        let unicode = flags.unicode || flags.v;
        let p = decodificar(padrao, unicode);
        let (total, nomes_pre) = prescan(&p)?;
        let mut an = Analisador {
            p,
            i: 0,
            unicode,
            total,
            contador: 0,
            nomes: nomes_pre,
            prof: 0,
            vset: flags.v,
        };
        let raiz = an.disjuncao()?;
        if an.i < an.p.len() {
            return Err("')' sem '(' correspondente".to_string());
        }
        if an.contador != total {
            return Err("contagem de grupos inconsistente".to_string());
        }
        let mut nomes = Vec::new();
        for (n, i) in &an.nomes {
            let mut u16s = Vec::new();
            pra_utf16(n, &mut u16s);
            nomes.push((u16s, *i));
        }
        let mut opc = flags.clone();
        opc.unicode = unicode;
        let mut raiz = raiz;
        preparar(&mut raiz, &opc);
        let prefixo = if flags.ignorar_caixa {
            None
        } else {
            prefixo_literal(&raiz)
        };
        let mut primeiros = [0u64; 4];
        let primeiros = match primeiros_de(&raiz, &opc, &mut primeiros) {
            Some(false) => Some(primeiros),
            _ => None,
        };
        Ok(Programa {
            raiz,
            grupos: total,
            nomes,
            opc,
            prefixo,
            primeiros,
        })
    }

    /// Número de grupos de captura (sem contar o grupo 0).
    pub fn grupos(&self) -> usize {
        self.grupos
    }

    /// Nomes dos grupos nomeados: (nome em UTF-16, índice do grupo, 1-based).
    pub fn nomes(&self) -> &[(Vec<u16>, usize)] {
        &self.nomes
    }

    /// Tenta casar começando EXATAMENTE em `inicio` (não procura adiante).
    pub fn casar_em(&self, texto: &[u16], inicio: usize) -> Option<Vec<Option<(usize, usize)>>> {
        let mut caps = vec![None; self.grupos + 1];
        self.casar_interno(texto, inicio, &mut 0, &mut caps).0.then_some(caps)
    }

    /// Procura o primeiro casamento a partir de `inicio` (se aderente, só em `inicio`).
    pub fn procurar(&self, texto: &[u16], inicio: usize) -> Option<Vec<Option<(usize, usize)>>> {
        if self.opc.aderente {
            return self.casar_em(texto, inicio);
        }
        self.procurar_adiante(texto, inicio)
    }

    /// Como `procurar`, ignorando a flag `y`: o primeiro casamento em `inicio` ou depois. Para o
    /// `split`, que tenta casar (aderente) em cada posição: dá o mesmo que essas tentativas uma a uma.
    pub fn procurar_adiante(&self, texto: &[u16], inicio: usize) -> Option<Vec<Option<(usize, usize)>>> {
        let mut passos = 0;
        let mut i = inicio;
        // Um vetor de capturas para a busca toda: cada tentativa só o limpa.
        let mut caps = vec![None; self.grupos + 1];
        while i <= texto.len() && passos <= LIMITE_BUSCA {
            if let Some(lit) = self.prefixo {
                // Pula direto para onde o primeiro caractere literal aparece.
                match texto.get(i..).and_then(|r| r.iter().position(|&u| u == lit)) {
                    Some(d) => i += d,
                    None => return None,
                }
            } else if let Some(ok) = &self.primeiros {
                match texto.get(i..).and_then(|r| r.iter().position(|&u| u < 256 && ok[u as usize >> 6] >> (u & 63) & 1 != 0)) {
                    Some(d) => i += d,
                    None => return None,
                }
            }
            let (achou, abortou) = self.casar_interno(texto, i, &mut passos, &mut caps);
            if achou {
                return Some(caps);
            }
            if abortou {
                return None;
            }
            i += match self.ler(texto, i) {
                Some((_, w)) => w,
                None => 1,
            };
        }
        None
    }

    /// `passos`: o que a busca já gastou (soma o desta tentativa).
    /// Devolve (casou, abortou); ao casar, as capturas ficam em `caps`.
    fn casar_interno(&self, t: &[u16], inicio: usize, passos: &mut u64, caps: &mut Vec<Option<(usize, usize)>>) -> (bool, bool) {
        if inicio > t.len() {
            return (false, false);
        }
        caps.fill(None);
        let mut e = Estado {
            caps: std::mem::take(caps),
            passos: 0,
            prof: 0,
            abortou: false,
        };
        let ok = self.m(&self.raiz, t, inicio, &mut e, &mut |p: usize, e2: &mut Estado| {
            e2.caps[0] = Some((inicio, p));
            true
        });
        *passos += e.passos;
        *caps = e.caps;
        (ok, !ok && e.abortou)
    }

    /// Casa `no` em `pos`; ao casar, chama a continuação `k`.
    fn m(
        &self,
        no: &No,
        t: &[u16],
        pos: usize,
        e: &mut Estado,
        k: &mut dyn FnMut(usize, &mut Estado) -> bool,
    ) -> bool {
        if !conta(e) {
            return false;
        }
        if e.prof >= LIMITE_PROFUNDIDADE {
            e.abortou = true;
            return false;
        }
        e.prof += 1;
        let r = self.m_interno(no, t, pos, e, k);
        e.prof -= 1;
        r
    }

    fn m_interno(
        &self,
        no: &No,
        t: &[u16],
        pos: usize,
        e: &mut Estado,
        k: &mut dyn FnMut(usize, &mut Estado) -> bool,
    ) -> bool {
        match no {
            No::Char(c) => match self.ler(t, pos) {
                Some((x, w)) if self.iguais(x, *c) => k(pos + w, e),
                _ => false,
            },
            No::Qualquer | No::Classe(_) => match self.atomo_casa(no, t, pos) {
                Some(np) => k(np, e),
                None => false,
            },
            No::Inicio => {
                let ok = pos == 0 || (self.opc.multilinha && terminador(t[pos - 1] as u32));
                if ok {
                    k(pos, e)
                } else {
                    false
                }
            }
            No::Fim => {
                let ok = pos == t.len() || (self.opc.multilinha && terminador(t[pos] as u32));
                if ok {
                    k(pos, e)
                } else {
                    false
                }
            }
            No::Fronteira(quer) => {
                let antes = pos > 0 && palavra(t[pos - 1]);
                let depois = pos < t.len() && palavra(t[pos]);
                if (antes != depois) == *quer {
                    k(pos, e)
                } else {
                    false
                }
            }
            No::Grupo(inner, 0) => self.m(inner, t, pos, e, k),
            No::Grupo(inner, idx) => {
                let idx = *idx;
                self.m(inner, t, pos, e, &mut |p: usize, e2: &mut Estado| {
                    let antes = e2.caps[idx];
                    e2.caps[idx] = Some((pos, p));
                    if k(p, e2) {
                        return true;
                    }
                    e2.caps[idx] = antes;
                    false
                })
            }
            No::Alt(opcoes) => {
                for o in opcoes {
                    if self.m(o, t, pos, e, k) {
                        return true;
                    }
                    if e.abortou {
                        return false;
                    }
                }
                false
            }
            No::Seq(v) => self.seq(v, 0, t, pos, e, k),
            No::Rep(inner, q) => {
                if matches!(**inner, No::Char(_) | No::Classe(_) | No::Qualquer) {
                    self.rep_atomo(inner, *q, t, pos, e, k)
                } else {
                    self.rep(inner, *q, 0, t, pos, e, k)
                }
            }
            No::Ref(idx) => {
                let cap = e.caps[*idx];
                match cap {
                    None => k(pos, e),
                    Some((s, f)) => match self.casa_trecho(&t[s..f], t, pos) {
                        Some(np) => k(np, e),
                        None => false,
                    },
                }
            }
            No::Olhar(inner, positivo, atras) => {
                let salvo = e.caps.clone();
                let achou = if *atras {
                    // shortcut: tenta todos os inícios para trás, quadrático.
                    let mut achou = false;
                    for j in (0..=pos).rev() {
                        if self.m(inner, t, j, e, &mut |p: usize, _e2: &mut Estado| p == pos) {
                            achou = true;
                            break;
                        }
                        if e.abortou {
                            return false;
                        }
                    }
                    achou
                } else {
                    self.m(inner, t, pos, e, &mut |_p: usize, _e2: &mut Estado| true)
                };
                if achou != *positivo {
                    e.caps = salvo;
                    return false;
                }
                if k(pos, e) {
                    return true;
                }
                e.caps = salvo;
                false
            }
        }
    }

    fn seq(
        &self,
        v: &[No],
        i: usize,
        t: &[u16],
        pos: usize,
        e: &mut Estado,
        k: &mut dyn FnMut(usize, &mut Estado) -> bool,
    ) -> bool {
        if i == v.len() {
            return k(pos, e);
        }
        self.m(&v[i], t, pos, e, &mut |p: usize, e2: &mut Estado| {
            self.seq(v, i + 1, t, p, e2, &mut *k)
        })
    }

    /// Quantificador guloso ou preguiçoso sobre átomo simples: laço iterativo.
    fn rep_atomo(
        &self,
        inner: &No,
        q: Quant,
        t: &[u16],
        pos: usize,
        e: &mut Estado,
        k: &mut dyn FnMut(usize, &mut Estado) -> bool,
    ) -> bool {
        if q.guloso && !self.opc.unicode {
            // Fora do modo u cada volta consome uma unidade: o fim da volta n é pos + n.
            let mx = q.max.unwrap_or(usize::MAX);
            let mut p = pos;
            while p - pos < mx {
                if !conta(e) {
                    return false;
                }
                match self.atomo_casa(inner, t, p) {
                    Some(np) => p = np,
                    None => break,
                }
            }
            let cont = p - pos;
            if cont < q.min {
                return false;
            }
            for n in (q.min..=cont).rev() {
                if !conta(e) {
                    return false;
                }
                if k(pos + n, e) {
                    return true;
                }
                if e.abortou {
                    return false;
                }
            }
            false
        } else if q.guloso {
            let mut fins = vec![pos];
            let mut p = pos;
            loop {
                if let Some(mx) = q.max {
                    if fins.len() - 1 >= mx {
                        break;
                    }
                }
                if !conta(e) {
                    return false;
                }
                match self.atomo_casa(inner, t, p) {
                    Some(np) => {
                        p = np;
                        fins.push(np);
                    }
                    None => break,
                }
            }
            let cont = fins.len() - 1;
            if cont < q.min {
                return false;
            }
            for n in (q.min..=cont).rev() {
                if !conta(e) {
                    return false;
                }
                if k(fins[n], e) {
                    return true;
                }
                if e.abortou {
                    return false;
                }
            }
            false
        } else {
            let mut p = pos;
            let mut n = 0usize;
            while n < q.min {
                if !conta(e) {
                    return false;
                }
                match self.atomo_casa(inner, t, p) {
                    Some(np) => {
                        p = np;
                        n += 1;
                    }
                    None => return false,
                }
            }
            loop {
                if k(p, e) {
                    return true;
                }
                if e.abortou {
                    return false;
                }
                if let Some(mx) = q.max {
                    if n >= mx {
                        return false;
                    }
                }
                if !conta(e) {
                    return false;
                }
                match self.atomo_casa(inner, t, p) {
                    Some(np) => {
                        p = np;
                        n += 1;
                    }
                    None => return false,
                }
            }
        }
    }

    /// Quantificador geral (grupo etc.): recursivo, uma volta por nível.
    #[allow(clippy::too_many_arguments)]
    fn rep(
        &self,
        inner: &No,
        q: Quant,
        cnt: usize,
        t: &[u16],
        pos: usize,
        e: &mut Estado,
        k: &mut dyn FnMut(usize, &mut Estado) -> bool,
    ) -> bool {
        if let Some(mx) = q.max {
            if cnt >= mx {
                return k(pos, e);
            }
        }
        if cnt < q.min {
            return self.iterar(inner, q, cnt, t, pos, e, k);
        }
        if q.guloso {
            if self.iterar(inner, q, cnt, t, pos, e, k) {
                return true;
            }
            if e.abortou {
                return false;
            }
            k(pos, e)
        } else {
            if k(pos, e) {
                return true;
            }
            if e.abortou {
                return false;
            }
            self.iterar(inner, q, cnt, t, pos, e, k)
        }
    }

    /// Uma volta do quantificador geral. Limpa as capturas de dentro (regra do JS).
    #[allow(clippy::too_many_arguments)]
    fn iterar(
        &self,
        inner: &No,
        q: Quant,
        cnt: usize,
        t: &[u16],
        pos: usize,
        e: &mut Estado,
        k: &mut dyn FnMut(usize, &mut Estado) -> bool,
    ) -> bool {
        let salvo: Vec<Option<(usize, usize)>> = e.caps[q.a..q.b].to_vec();
        for c in e.caps[q.a..q.b].iter_mut() {
            *c = None;
        }
        let ok = self.m(inner, t, pos, e, &mut |p: usize, e2: &mut Estado| {
            // Volta que não consumiu nada depois do mínimo: falha (evita laço infinito).
            if cnt >= q.min && p == pos {
                return false;
            }
            self.rep(inner, q, cnt + 1, t, p, e2, &mut *k)
        });
        if !ok {
            e.caps[q.a..q.b].clone_from_slice(&salvo);
        }
        ok
    }

    /// Consome um átomo simples em `pos`; devolve a nova posição.
    fn atomo_casa(&self, no: &No, t: &[u16], pos: usize) -> Option<usize> {
        let (x, w) = self.ler(t, pos)?;
        let ok = match no {
            No::Char(c) => self.iguais(x, *c),
            No::Qualquer => self.opc.ponto_tudo || !terminador(x),
            No::Classe(cl) => self.classe_casa(cl, x),
            _ => false,
        };
        if ok {
            Some(pos + w)
        } else {
            None
        }
    }

    /// Lê um caractere em `i` (um par substituto conta como um só no modo u).
    /// Devolve (ponto de código, unidades ocupadas).
    fn ler(&self, t: &[u16], i: usize) -> Option<(u32, usize)> {
        let u = *t.get(i)? as u32;
        if self.opc.unicode && (0xD800..0xDC00).contains(&u) {
            if let Some(&v) = t.get(i + 1) {
                let v = v as u32;
                if (0xDC00..0xE000).contains(&v) {
                    return Some((0x10000 + ((u - 0xD800) << 10) + (v - 0xDC00), 2));
                }
            }
        }
        Some((u, 1))
    }

    fn iguais(&self, a: u32, b: u32) -> bool {
        a == b || (self.opc.ignorar_caixa && baixa(a) == baixa(b))
    }

    fn classe_casa(&self, cl: &Classe, c: u32) -> bool {
        if c < 256 {
            return cl.mapa[c as usize >> 6] >> (c & 63) & 1 != 0;
        }
        self.classe_casa_lento(cl, c)
    }

    fn classe_casa_lento(&self, cl: &Classe, c: u32) -> bool {
        let achou = if self.opc.ignorar_caixa {
            [c, baixa(c), alta(c)].iter().any(|&x| cl.itens_casa(x))
        } else {
            cl.itens_casa(c)
        };
        achou != cl.negada
    }

    /// Casa o texto `sub` (retro-referência) em `pos`; devolve a nova posição.
    fn casa_trecho(&self, sub: &[u16], t: &[u16], pos: usize) -> Option<usize> {
        let mut i = 0;
        let mut j = pos;
        while i < sub.len() {
            let (a, wa) = self.ler(sub, i)?;
            let (b, wb) = self.ler(t, j)?;
            if !self.iguais(a, b) {
                return None;
            }
            i += wa;
            j += wb;
        }
        Some(j)
    }
}

/// Analisador do padrão. Trabalha sobre pontos de código do padrão.
struct Analisador {
    p: Vec<u32>,
    i: usize,
    unicode: bool,
    /// Total de grupos de captura (vem da pré-leitura).
    total: usize,
    /// Grupos já abertos no texto (define o índice do próximo).
    contador: usize,
    nomes: Vec<(Vec<u32>, usize)>,
    /// Grupos abertos agora: `((((…` fundo estouraria a pilha do Rust na leitura.
    prof: usize,
    /// Modo `v`: classes com conjuntos (`--`, `&&`, aninhadas, `\q{..}`).
    vset: bool,
}

/// Item de classe ainda sem faixa montada.
enum Ac {
    C(u32),
    P(Pred, bool),
}

fn push_ac(itens: &mut Vec<Item>, a: Ac) {
    match a {
        Ac::C(c) => itens.push(Item::Faixa(c, c)),
        Ac::P(p, n) => itens.push(Item::Pred(p, n)),
    }
}

impl Analisador {
    fn olha(&self) -> Option<u32> {
        self.p.get(self.i).copied()
    }

    fn olha_em(&self, d: usize) -> Option<u32> {
        self.p.get(self.i + d).copied()
    }

    fn avanca(&mut self) -> Option<u32> {
        let c = self.olha()?;
        self.i += 1;
        Some(c)
    }

    fn e_char(&self, c: char) -> bool {
        self.olha() == Some(c as u32)
    }

    fn fecha(&mut self) -> Result<(), String> {
        if self.e_char(')') {
            self.i += 1;
            Ok(())
        } else {
            Err("parênteses não fechados".to_string())
        }
    }

    fn disjuncao(&mut self) -> Result<No, String> {
        let mut alts = vec![self.alternativa()?];
        while self.e_char('|') {
            self.i += 1;
            alts.push(self.alternativa()?);
        }
        if alts.len() == 1 {
            Ok(alts.remove(0))
        } else {
            Ok(No::Alt(alts))
        }
    }

    fn alternativa(&mut self) -> Result<No, String> {
        let mut v = Vec::new();
        while let Some(c) = self.olha() {
            if c == '|' as u32 || c == ')' as u32 {
                break;
            }
            let t = self.termo()?;
            v.push(t);
        }
        Ok(No::Seq(v))
    }

    fn termo(&mut self) -> Result<No, String> {
        let g0 = self.contador;
        let (atomo, quantificavel) = self.atomo()?;
        let g1 = self.contador;
        let (min, max) = match self.quantificador()? {
            Some(q) => q,
            None => return Ok(atomo),
        };
        if !quantificavel {
            return Err("nada para repetir".to_string());
        }
        let guloso = if self.e_char('?') {
            self.i += 1;
            false
        } else {
            true
        };
        Ok(No::Rep(
            Box::new(atomo),
            Quant {
                min,
                max,
                guloso,
                a: g0 + 1,
                b: g1 + 1,
            },
        ))
    }

    /// Lê `*`, `+`, `?` ou `{n,m}`. Em `{` malformado: erro no modo u, literal no modo sem u.
    fn quantificador(&mut self) -> Result<Option<(usize, Option<usize>)>, String> {
        let save = self.i;
        let c = match self.olha() {
            Some(c) => c,
            None => return Ok(None),
        };
        let simples: Option<(usize, Option<usize>)> = if c == '*' as u32 {
            Some((0, None))
        } else if c == '+' as u32 {
            Some((1, None))
        } else if c == '?' as u32 {
            Some((0, Some(1)))
        } else {
            None
        };
        if let Some(q) = simples {
            self.i += 1;
            return Ok(Some(q));
        }
        if c != '{' as u32 {
            return Ok(None);
        }
        self.i += 1;
        let mut ok: Option<(usize, Option<usize>)> = None;
        if let Some(mn) = self.numero() {
            if self.e_char('}') {
                self.i += 1;
                ok = Some((mn, Some(mn)));
            } else if self.e_char(',') {
                self.i += 1;
                if self.e_char('}') {
                    self.i += 1;
                    ok = Some((mn, None));
                } else if let Some(mx) = self.numero() {
                    if self.e_char('}') {
                        self.i += 1;
                        ok = Some((mn, Some(mx)));
                    }
                }
            }
        }
        match ok {
            Some((mn, Some(mx))) if mx < mn => {
                Err("intervalo de quantificador fora de ordem".to_string())
            }
            Some(q) => Ok(Some(q)),
            None => {
                if self.unicode {
                    Err("quantificador malformado".to_string())
                } else {
                    self.i = save;
                    Ok(None)
                }
            }
        }
    }

    fn numero(&mut self) -> Option<usize> {
        let inicio = self.i;
        let mut v: usize = 0;
        while let Some(c) = self.olha() {
            if !(0x30..=0x39).contains(&c) {
                break;
            }
            v = v.saturating_mul(10).saturating_add((c - 0x30) as usize);
            self.i += 1;
        }
        if self.i == inicio {
            None
        } else {
            Some(v)
        }
    }

    /// Lê um átomo. Devolve (nó, se pode receber quantificador).
    fn atomo(&mut self) -> Result<(No, bool), String> {
        let c = self
            .avanca()
            .ok_or_else(|| "padrão inesperado".to_string())?;
        let ch = char::from_u32(c).unwrap_or('\0');
        match ch {
            '^' => Ok((No::Inicio, false)),
            '$' => Ok((No::Fim, false)),
            '.' => Ok((No::Qualquer, true)),
            '(' => self.grupo(),
            '[' => {
                if self.vset {
                    let c = self.classe_v()?;
                    return Ok((conj_no(c), true));
                }
                let cl = self.classe()?;
                Ok((No::Classe(cl), true))
            }
            '\\' => self.escape_fora(),
            '*' | '+' | '?' => Err("nada para repetir".to_string()),
            '{' => {
                if self.unicode {
                    return Err("'{' solto".to_string());
                }
                // Sem u: '{' que não forma quantificador é literal.
                self.i -= 1;
                if self.quantificador()?.is_some() {
                    return Err("nada para repetir".to_string());
                }
                self.i += 1;
                Ok((No::Char(c), true))
            }
            '}' | ']' if self.unicode => Err(format!("'{}' solto", ch)),
            _ => Ok((No::Char(c), true)),
        }
    }

    /// Grupo; o `(` já foi consumido.
    fn grupo(&mut self) -> Result<(No, bool), String> {
        if self.prof >= LIMITE_PROFUNDIDADE {
            return Err("grupos aninhados demais".to_string());
        }
        self.prof += 1;
        let r = self.grupo_nivel();
        self.prof -= 1;
        r
    }

    fn grupo_nivel(&mut self) -> Result<(No, bool), String> {
        if !self.e_char('?') {
            self.contador += 1;
            let idx = self.contador;
            let d = self.disjuncao()?;
            self.fecha()?;
            return Ok((No::Grupo(Box::new(d), idx), true));
        }
        self.i += 1;
        let c = self
            .avanca()
            .ok_or_else(|| "grupo não fechado".to_string())?;
        if c == ':' as u32 {
            let d = self.disjuncao()?;
            self.fecha()?;
            return Ok((No::Grupo(Box::new(d), 0), true));
        }
        if c == '=' as u32 || c == '!' as u32 {
            let positivo = c == '=' as u32;
            let d = self.disjuncao()?;
            self.fecha()?;
            // Sem u, o lookahead aceita quantificador (Anexo B).
            return Ok((No::Olhar(Box::new(d), positivo, false), !self.unicode));
        }
        if c != '<' as u32 {
            return Err("grupo inválido".to_string());
        }
        if self.e_char('=') || self.e_char('!') {
            let positivo = self.e_char('=');
            self.i += 1;
            let d = self.disjuncao()?;
            self.fecha()?;
            return Ok((No::Olhar(Box::new(d), positivo, true), false));
        }
        // Grupo nomeado: (?<nome>...)
        self.ler_nome()?;
        self.contador += 1;
        let idx = self.contador;
        let d = self.disjuncao()?;
        self.fecha()?;
        Ok((No::Grupo(Box::new(d), idx), true))
    }

    /// Lê `nome>` (o `<` já foi consumido) e valida o nome.
    fn ler_nome(&mut self) -> Result<Vec<u32>, String> {
        let mut raw = Vec::new();
        loop {
            match self.avanca() {
                None => return Err("nome de grupo sem '>'".to_string()),
                Some(c) if c == '>' as u32 => break,
                Some(c) => raw.push(c),
            }
        }
        match decodifica_nome(&raw) {
            Some(n) if !n.is_empty() && n.iter().enumerate().all(|(k, &c)| id_nome(c, k == 0)) => Ok(n),
            _ => Err("nome de grupo inválido".to_string()),
        }
    }

    /// Escape fora de classe; a barra já foi consumida.
    fn escape_fora(&mut self) -> Result<(No, bool), String> {
        let c = self
            .avanca()
            .ok_or_else(|| "'\\' no fim do padrão".to_string())?;
        let ch = char::from_u32(c).unwrap_or('\0');
        match ch {
            'b' => Ok((No::Fronteira(true), false)),
            'B' => Ok((No::Fronteira(false), false)),
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                let (p, neg) = pred_de_letra(ch);
                Ok((
                    No::Classe(Classe {
                        itens: vec![Item::Pred(p, neg)],
                        negada: false,
                        mapa: [0; 4],
                    }),
                    true,
                ))
            }
            'p' | 'P' if self.unicode => {
                let pr = self.ler_propriedade()?;
                let neg = ch == 'P';
                Ok((
                    No::Classe(Classe {
                        itens: vec![Item::Pred(Pred::Tab(pr), neg)],
                        negada: false,
                        mapa: [0; 4],
                    }),
                    true,
                ))
            }
            '1'..='9' => {
                let inicio = self.i - 1;
                self.i = inicio;
                let n = self.numero().unwrap_or(0);
                if n <= self.total {
                    return Ok((No::Ref(n), true));
                }
                if self.unicode {
                    return Err("referência a grupo inexistente".to_string());
                }
                Ok((No::Char(self.octal_legado(inicio)), true))
            }
            '0' => {
                if self.olha().map_or(false, |x| (0x30..=0x39).contains(&x)) {
                    if self.unicode {
                        return Err("\\0 seguido de dígito".to_string());
                    }
                    let inicio = self.i - 1;
                    Ok((No::Char(self.octal_legado(inicio)), true))
                } else {
                    Ok((No::Char(0), true))
                }
            }
            'k' if self.unicode || !self.nomes.is_empty() => {
                if !self.e_char('<') {
                    return Err("\\k sem nome de grupo".to_string());
                }
                self.i += 1;
                let nome = self.ler_nome()?;
                let idx = self
                    .nomes
                    .iter()
                    .find(|(n, _)| *n == nome)
                    .map(|(_, i)| *i);
                match idx {
                    Some(i) => Ok((No::Ref(i), true)),
                    None => Err("nome de grupo inexistente".to_string()),
                }
            }
            _ => Ok((No::Char(self.escape_simples(c)?), true)),
        }
    }

    /// Escape de caractere (\t, \cX, \xHH, \uHHHH, identidade). `c` já foi consumido.
    fn escape_simples(&mut self, c: u32) -> Result<u32, String> {
        let ch = char::from_u32(c).unwrap_or('\0');
        let v = match ch {
            't' => 9,
            'n' => 10,
            'v' => 11,
            'f' => 12,
            'r' => 13,
            'c' => match self.olha().and_then(char::from_u32) {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.i += 1;
                    (l as u32) % 32
                }
                _ => {
                    if self.unicode {
                        return Err("\\c sem letra".to_string());
                    }
                    // Sem u: a barra vira literal e o 'c' é lido depois.
                    self.i -= 1;
                    '\\' as u32
                }
            },
            'x' => match self.hex(2) {
                Some(v) => v,
                None => {
                    if self.unicode {
                        return Err("\\x inválido".to_string());
                    }
                    'x' as u32
                }
            },
            'u' => self.escape_u()?,
            _ => {
                if self.unicode && !"^$\\.*+?()[]{}|/-".contains(ch) {
                    return Err(format!("escape de identidade inválido: \\{}", ch));
                }
                c
            }
        };
        Ok(v)
    }

    /// `\uHHHH`, `\u{H...}` (modo u) e pares substitutos `😀` (modo u).
    fn escape_u(&mut self) -> Result<u32, String> {
        if self.unicode && self.e_char('{') {
            self.i += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            while let Some(dig) = self
                .olha()
                .and_then(char::from_u32)
                .and_then(|c| c.to_digit(16))
            {
                v = v.saturating_mul(16).saturating_add(dig);
                self.i += 1;
                n += 1;
            }
            if n == 0 || !self.e_char('}') || v > 0x10FFFF {
                return Err("\\u{...} inválido".to_string());
            }
            self.i += 1;
            return Ok(v);
        }
        match self.hex(4) {
            Some(v) => {
                if self.unicode
                    && (0xD800..0xDC00).contains(&v)
                    && self.olha() == Some('\\' as u32)
                    && self.olha_em(1) == Some('u' as u32)
                {
                    let salvo = self.i;
                    self.i += 2;
                    match self.hex(4) {
                        Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                            return Ok(0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00));
                        }
                        _ => self.i = salvo,
                    }
                }
                Ok(v)
            }
            None => {
                if self.unicode {
                    Err("\\u inválido".to_string())
                } else {
                    Ok('u' as u32)
                }
            }
        }
    }

    /// Lê exatamente `n` dígitos hexadecimais; não consome nada se faltar algum.
    fn hex(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for k in 0..n {
            let d = self.olha_em(k)?;
            let dig = char::from_u32(d)?.to_digit(16)?;
            v = v * 16 + dig;
        }
        self.i += n;
        Some(v)
    }

    /// Octal legado (Anexo B) ou '8'/'9' literais. `inicio` aponta para o primeiro dígito.
    fn octal_legado(&mut self, inicio: usize) -> u32 {
        self.i = inicio;
        let primeiro = self.p[inicio];
        if primeiro == '8' as u32 || primeiro == '9' as u32 {
            self.i += 1;
            return primeiro;
        }
        let mut v = 0u32;
        let mut n = 0;
        while n < 3 {
            match self.olha() {
                Some(x) if (0x30..=0x37).contains(&x) && v * 8 + (x - 0x30) <= 255 => {
                    v = v * 8 + (x - 0x30);
                    self.i += 1;
                    n += 1;
                }
                _ => break,
            }
        }
        v
    }

    /// `{Nome}` depois de \p ou \P (modo u).
    fn ler_propriedade(&mut self) -> Result<&'static [(u32, u32)], String> {
        if !self.e_char('{') {
            return Err("\\p exige {nome}".to_string());
        }
        self.i += 1;
        let mut nome = String::new();
        loop {
            let c = self
                .avanca()
                .ok_or_else(|| "\\p sem '}'".to_string())?;
            if c == '}' as u32 {
                break;
            }
            match char::from_u32(c) {
                Some(ch) if ch.is_ascii() => nome.push(ch),
                _ => return Err("nome de propriedade inválido".to_string()),
            }
        }
        prop_de_nome(&nome).ok_or_else(|| format!("propriedade Unicode desconhecida: {}", nome))
    }

    /// Classe `[...]`; o `[` já foi consumido.
    fn classe(&mut self) -> Result<Classe, String> {
        let negada = self.e_char('^');
        if negada {
            self.i += 1;
        }
        let mut itens = Vec::new();
        loop {
            if self.olha().is_none() {
                return Err("classe não fechada".to_string());
            }
            if self.e_char(']') {
                self.i += 1;
                break;
            }
            let a = self.item_classe()?;
            let prox = self.olha_em(1);
            if self.e_char('-') && prox.is_some() && prox != Some(']' as u32) {
                self.i += 1;
                let b = self.item_classe()?;
                match (a, b) {
                    (Ac::C(x), Ac::C(y)) => {
                        if x > y {
                            return Err("intervalo fora de ordem".to_string());
                        }
                        itens.push(Item::Faixa(x, y));
                    }
                    (a2, b2) => {
                        if self.unicode {
                            return Err("intervalo inválido na classe".to_string());
                        }
                        push_ac(&mut itens, a2);
                        itens.push(Item::Faixa('-' as u32, '-' as u32));
                        push_ac(&mut itens, b2);
                    }
                }
            } else {
                push_ac(&mut itens, a);
            }
        }
        Ok(Classe { itens, negada, mapa: [0; 4] })
    }

    /// Classe do modo `v` (o `[` já foi consumido): união, `--` ou `&&`, aninhadas, `\q{..}`.
    fn classe_v(&mut self) -> Result<Conj, String> {
        self.prof += 1;
        if self.prof > LIMITE_PROFUNDIDADE {
            return Err("classes aninhadas demais".to_string());
        }
        let negada = self.e_char('^');
        if negada {
            self.i += 1;
        }
        let mut acc = Conj::default();
        if !self.e_char(']') {
            let primeiro = self.operando_v()?;
            let op = if self.duplo('-') {
                Some('-')
            } else if self.duplo('&') {
                Some('&')
            } else {
                None
            };
            match op {
                Some(o) => {
                    acc = normaliza_conj(primeiro.0);
                    loop {
                        if self.e_char(']') {
                            self.i += 1;
                            break;
                        }
                        if !self.duplo(o) || (o == '&' && self.olha_em(2) == Some('&' as u32)) {
                            return Err("operador de classe inválido ou misturado".to_string());
                        }
                        self.i += 2;
                        let (c2, _) = self.operando_v()?;
                        let c2 = normaliza_conj(c2);
                        acc = if o == '-' { subtrai_conj(acc, c2) } else { intersecao_conj(acc, c2) };
                    }
                }
                None => {
                    let mut pend = Some(primeiro);
                    loop {
                        let (c, ch) = match pend.take() {
                            Some(x) => x,
                            None => {
                                if self.e_char(']') {
                                    self.i += 1;
                                    break;
                                }
                                if self.duplo('-') || self.duplo('&') {
                                    return Err("operador de classe inválido ou misturado".to_string());
                                }
                                self.operando_v()?
                            }
                        };
                        match ch {
                            Some(x) if self.e_char('-') => {
                                if self.olha_em(1) == Some('-' as u32) {
                                    return Err("operador '--' misturado com união".to_string());
                                }
                                self.i += 1;
                                let y = self
                                    .operando_v()?
                                    .1
                                    .ok_or_else(|| "intervalo inválido na classe".to_string())?;
                                if x > y {
                                    return Err("intervalo fora de ordem".to_string());
                                }
                                acc.faixas.push((x, y));
                            }
                            _ => {
                                acc.faixas.extend(c.faixas);
                                acc.cadeias.extend(c.cadeias);
                            }
                        }
                    }
                }
            }
        }
        acc.faixas = normaliza(acc.faixas);
        if negada {
            if !acc.cadeias.is_empty() {
                return Err("classe negada não pode ter cadeias \\q".to_string());
            }
            acc.faixas = complemento(&acc.faixas);
        }
        self.prof -= 1;
        Ok(acc)
    }

    /// `--`/`&&` na posição atual?
    fn duplo(&self, c: char) -> bool {
        self.e_char(c) && self.olha_em(1) == Some(c as u32)
    }

    /// Operando do modo `v`: classe aninhada, `\q{..}`, classe de escape ou caractere.
    /// Devolve o conjunto e, se for um caractere solto, esse caractere (para `a-z`).
    fn operando_v(&mut self) -> Result<(Conj, Option<u32>), String> {
        let c = self.olha().ok_or_else(|| "classe não fechada".to_string())?;
        if c == '[' as u32 {
            self.i += 1;
            return Ok((self.classe_v()?, None));
        }
        if c == '\\' as u32 && self.olha_em(1) == Some('q' as u32) {
            self.i += 2;
            return Ok((self.q_v()?, None));
        }
        if c != '\\' as u32 {
            let ch = char::from_u32(c).unwrap_or('\0');
            if "()[]{}/-|".contains(ch) {
                return Err(format!("'{}' precisa de escape na classe", ch));
            }
            if "&!#$%*+,.:;<=>?@^`~".contains(ch) && self.olha_em(1) == Some(c) {
                return Err("pontuador duplo reservado na classe".to_string());
            }
        }
        Ok(match self.item_classe()? {
            Ac::C(x) => (Conj { faixas: vec![(x, x)], cadeias: Vec::new() }, Some(x)),
            Ac::P(p, n) => (Conj { faixas: pred_faixas(p, n), cadeias: Vec::new() }, None),
        })
    }

    /// `{a|bc|}` depois de `\q`: um caractere vira faixa, o resto vira cadeia.
    fn q_v(&mut self) -> Result<Conj, String> {
        if !self.e_char('{') {
            return Err("\\q exige {".to_string());
        }
        self.i += 1;
        let mut conj = Conj::default();
        let mut cad: Vec<u32> = Vec::new();
        loop {
            let c = self.olha().ok_or_else(|| "\\q sem '}'".to_string())?;
            if c == '}' as u32 || c == '|' as u32 {
                self.i += 1;
                if cad.len() == 1 {
                    conj.faixas.push((cad[0], cad[0]));
                } else {
                    conj.cadeias.push(std::mem::take(&mut cad));
                }
                if c == '}' as u32 {
                    break;
                }
                cad.clear();
                continue;
            }
            match self.item_classe()? {
                Ac::C(x) => cad.push(x),
                Ac::P(..) => return Err("\\q não aceita classe de escape".to_string()),
            }
        }
        conj.faixas = normaliza(conj.faixas);
        Ok(conj)
    }

    /// Um item dentro de classe (caractere ou escape).
    fn item_classe(&mut self) -> Result<Ac, String> {
        let c = self
            .avanca()
            .ok_or_else(|| "classe não fechada".to_string())?;
        if c != '\\' as u32 {
            return Ok(Ac::C(c));
        }
        let d = self
            .avanca()
            .ok_or_else(|| "'\\' no fim do padrão".to_string())?;
        let ch = char::from_u32(d).unwrap_or('\0');
        match ch {
            'b' => Ok(Ac::C(8)),
            'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                let (p, n) = pred_de_letra(ch);
                Ok(Ac::P(p, n))
            }
            'p' | 'P' if self.unicode => {
                let pr = self.ler_propriedade()?;
                Ok(Ac::P(Pred::Tab(pr), ch == 'P'))
            }
            '0'..='9' => {
                if ch == '0' && !self.olha().map_or(false, |x| (0x30..=0x39).contains(&x)) {
                    return Ok(Ac::C(0));
                }
                if self.unicode {
                    return Err("escape numérico inválido na classe".to_string());
                }
                let inicio = self.i - 1;
                Ok(Ac::C(self.octal_legado(inicio)))
            }
            _ => Ok(Ac::C(self.escape_simples(d)?)),
        }
    }
}

/// Conjunto do modo `v`: faixas normalizadas e cadeias de `\q{..}` de tamanho != 1.
#[derive(Default)]
struct Conj {
    faixas: Vec<(u32, u32)>,
    cadeias: Vec<Vec<u32>>,
}

/// Ordena e funde faixas que se tocam ou se sobrepõem.
fn normaliza(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    v.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1.saturating_add(1) => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn normaliza_conj(c: Conj) -> Conj {
    Conj { faixas: normaliza(c.faixas), cadeias: c.cadeias }
}

/// Complemento em 0..=0x10FFFF de faixas normalizadas.
fn complemento(v: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut prox = 0u32;
    for &(a, b) in v {
        if a > prox {
            out.push((prox, a - 1));
        }
        prox = b + 1;
    }
    if prox <= 0x10FFFF {
        out.push((prox, 0x10FFFF));
    }
    out
}

/// Interseção de duas listas de faixas normalizadas.
fn intersecao(a: &[(u32, u32)], b: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo <= hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn intersecao_conj(a: Conj, b: Conj) -> Conj {
    let faixas = intersecao(&a.faixas, &b.faixas);
    let cadeias = a.cadeias.into_iter().filter(|s| b.cadeias.contains(s)).collect();
    Conj { faixas, cadeias }
}

fn subtrai_conj(a: Conj, b: Conj) -> Conj {
    let faixas = intersecao(&a.faixas, &complemento(&b.faixas));
    let cadeias = a.cadeias.into_iter().filter(|s| !b.cadeias.contains(s)).collect();
    Conj { faixas, cadeias }
}

/// Faixas de um predicado (\d \w \s \p{..}), já complementadas se `neg`.
fn pred_faixas(p: Pred, neg: bool) -> Vec<(u32, u32)> {
    let v = match p {
        Pred::Dig => vec![(0x30, 0x39)],
        Pred::Pal => vec![(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)],
        Pred::Esp => vec![
            (9, 13), (0x20, 0x20), (0xA0, 0xA0), (0x1680, 0x1680), (0x2000, 0x200A),
            (0x2028, 0x2029), (0x202F, 0x202F), (0x205F, 0x205F), (0x3000, 0x3000), (0xFEFF, 0xFEFF),
        ],
        Pred::Tab(t) => t.to_vec(),
    };
    let v = normaliza(v);
    if neg { complemento(&v) } else { v }
}

/// Conjunto do modo `v` -> nó: alternativas das cadeias (a mais longa primeiro), depois a classe.
fn conj_no(c: Conj) -> No {
    let cl = No::Classe(Classe {
        itens: c.faixas.iter().map(|&(a, b)| Item::Faixa(a, b)).collect(),
        negada: false,
        mapa: [0; 4],
    });
    if c.cadeias.is_empty() {
        return cl;
    }
    let mut cads = c.cadeias;
    cads.sort_by(|a, b| b.len().cmp(&a.len()));
    let mut alts: Vec<No> = Vec::new();
    let mut vazia = false;
    for s in cads {
        if s.is_empty() {
            vazia = true;
        } else {
            alts.push(No::Seq(s.into_iter().map(No::Char).collect()));
        }
    }
    alts.push(cl);
    if vazia {
        alts.push(No::Seq(Vec::new()));
    }
    No::Alt(alts)
}

/// `\d \D \w \W \s \S` -> (predicado, negado).
fn pred_de_letra(ch: char) -> (Pred, bool) {
    match ch {
        'd' => (Pred::Dig, false),
        'D' => (Pred::Dig, true),
        'w' => (Pred::Pal, false),
        'W' => (Pred::Pal, true),
        's' => (Pred::Esp, false),
        'S' => (Pred::Esp, true),
        _ => (Pred::Esp, false),
    }
}

/// Nome de `\p{..}` -> faixas Unicode oficiais (General_Category, Script, Script_Extensions
/// ou propriedade binária). Nomes exatos, sem casamento frouxo, como manda o ECMAScript.
fn prop_de_nome(nome: &str) -> Option<&'static [(u32, u32)]> {
    use crate::unicode_tabelas as ut;
    for (pre, ext) in [("Script=", false), ("sc=", false), ("Script_Extensions=", true), ("scx=", true)] {
        if let Some(v) = nome.strip_prefix(pre) {
            return ut::SCRIPTS.iter().find(|(al, _, _)| al.contains(&v)).map(|(_, sc, scx)| if ext { *scx } else { *sc });
        }
    }
    for pre in ["General_Category=", "gc="] {
        if let Some(v) = nome.strip_prefix(pre) {
            return ut::CATEGORIAS.iter().find(|(al, _)| al.contains(&v)).map(|(_, f)| *f);
        }
    }
    if nome.contains('=') {
        return None;
    }
    ut::CATEGORIAS
        .iter()
        .find(|(al, _)| al.contains(&nome))
        .or_else(|| ut::BINARIAS.iter().find(|(al, _)| al.contains(&nome)))
        .map(|(_, f)| *f)
}

impl Pred {
    fn casa(self, x: u32) -> bool {
        match self {
            Pred::Dig => (0x30..=0x39).contains(&x),
            Pred::Pal => x < 128 && ((x as u8).is_ascii_alphanumeric() || x == 0x5F),
            Pred::Esp => matches!(
                x,
                9..=13
                    | 32
                    | 0xA0
                    | 0x1680
                    | 0x2000..=0x200A
                    | 0x2028
                    | 0x2029
                    | 0x202F
                    | 0x205F
                    | 0x3000
                    | 0xFEFF
            ),
            Pred::Tab(t) => em_faixas(t, x),
        }
    }
}

impl Item {
    fn casa(self, x: u32) -> bool {
        match self {
            Item::Faixa(a, b) => a <= x && x <= b,
            Item::Pred(p, neg) => p.casa(x) != neg,
        }
    }
}

impl Classe {
    fn itens_casa(&self, c: u32) -> bool {
        self.itens.iter().any(|it| it.casa(c))
    }
}

/// Minúscula de um caractere, se for um único caractere; senão o próprio.
fn baixa(c: u32) -> u32 {
    match char::from_u32(c) {
        Some(ch) => {
            let mut it = ch.to_lowercase();
            match (it.next(), it.next()) {
                (Some(x), None) => x as u32,
                _ => c,
            }
        }
        None => c,
    }
}

/// Maiúscula de um caractere, se for um único caractere; senão o próprio.
fn alta(c: u32) -> u32 {
    match char::from_u32(c) {
        Some(ch) => {
            let mut it = ch.to_uppercase();
            match (it.next(), it.next()) {
                (Some(x), None) => x as u32,
                _ => c,
            }
        }
        None => c,
    }
}

fn terminador(c: u32) -> bool {
    c == 0x0A || c == 0x0D || c == 0x2028 || c == 0x2029
}

fn palavra(c: u16) -> bool {
    let c = c as u32;
    c < 128 && ((c as u8).is_ascii_alphanumeric() || c == 0x5F)
}

/// Padrão em UTF-16 -> pontos de código (no modo u, pares viram um só).
fn decodificar(p: &[u16], unicode: bool) -> Vec<u32> {
    let mut v = Vec::with_capacity(p.len());
    let mut i = 0;
    while i < p.len() {
        let u = p[i] as u32;
        if unicode
            && (0xD800..0xDC00).contains(&u)
            && i + 1 < p.len()
            && (0xDC00..0xE000).contains(&(p[i + 1] as u32))
        {
            v.push(0x10000 + ((u - 0xD800) << 10) + ((p[i + 1] as u32) - 0xDC00));
            i += 2;
        } else {
            v.push(u);
            i += 1;
        }
    }
    v
}

/// Pontos de código -> UTF-16.
fn pra_utf16(cps: &[u32], out: &mut Vec<u16>) {
    for &c in cps {
        if c >= 0x10000 {
            let v = c - 0x10000;
            out.push((0xD800 + (v >> 10)) as u16);
            out.push((0xDC00 + (v & 0x3FF)) as u16);
        } else {
            out.push(c as u16);
        }
    }
}

fn ch(c: char) -> u32 {
    c as u32
}

/// Pré-leitura: conta os grupos de captura e recolhe os nomes (1-based).
/// Precisa do total antes de analisar, para decidir `\N` (retro-referência ou octal).
/// Nome de grupo: decodifica `\uXXXX`, `\u{..}` e pares substitutos (com ou sem `u`).
fn decodifica_nome(raw: &[u32]) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let (mut cp, n) = if raw[i] == ch('\\') {
            ler_escape_u(raw, i)?
        } else {
            (raw[i], 1)
        };
        i += n;
        let alto = (0xD800..0xDC00).contains(&cp);
        if alto {
            if let Some((lo, m)) = raw.get(i).filter(|&&c| c == ch('\\')).and_then(|_| ler_escape_u(raw, i)) {
                if (0xDC00..0xE000).contains(&lo) {
                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    i += m;
                }
            } else if let Some(&lo) = raw.get(i).filter(|&&c| (0xDC00..0xE000).contains(&c)) {
                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                i += 1;
            }
        }
        out.push(cp);
    }
    Some(out)
}

/// `\uXXXX` ou `\u{..}` em `raw[i..]`: (ponto de código, unidades consumidas).
fn ler_escape_u(raw: &[u32], i: usize) -> Option<(u32, usize)> {
    if raw.get(i) != Some(&ch('\\')) || raw.get(i + 1) != Some(&ch('u')) {
        return None;
    }
    if raw.get(i + 2) == Some(&ch('{')) {
        let mut j = i + 3;
        let mut v = 0u32;
        loop {
            let c = *raw.get(j)?;
            if c == ch('}') {
                break;
            }
            v = v.checked_mul(16)?.checked_add(char::from_u32(c)?.to_digit(16)?)?;
            if v > 0x10FFFF {
                return None;
            }
            j += 1;
        }
        return (j > i + 3).then_some((v, j + 1 - i));
    }
    let mut v = 0u32;
    for k in 0..4 {
        v = v * 16 + char::from_u32(*raw.get(i + 2 + k)?)?.to_digit(16)?;
    }
    Some((v, 6))
}

/// Caractere válido em nome de grupo (IdentifierStart / IdentifierPart do Unicode).
fn id_nome(c: u32, inicio: bool) -> bool {
    if c == ch('$') || c == ch('_') {
        return true;
    }
    let tab = |nome: &str| {
        crate::unicode_tabelas::BINARIAS.iter().find(|(al, _)| al.contains(&nome)).map_or(&[][..], |(_, f)| *f)
    };
    if inicio {
        em_faixas(tab("ID_Start"), c)
    } else {
        c == 0x200C || c == 0x200D || em_faixas(tab("ID_Continue"), c)
    }
}

/// Busca binária em faixas ordenadas.
fn em_faixas(t: &[(u32, u32)], x: u32) -> bool {
    t.binary_search_by(|&(a, b)| {
        if b < x {
            std::cmp::Ordering::Less
        } else if a > x {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

fn prescan(p: &[u32]) -> Result<(usize, Vec<(Vec<u32>, usize)>), String> {
    let mut total = 0usize;
    let mut nomes: Vec<(Vec<u32>, usize)> = Vec::new();
    let mut em_classe = false;
    let mut i = 0usize;
    while i < p.len() {
        let c = p[i];
        if c == ch('\\') {
            i += 2;
            continue;
        }
        if em_classe {
            if c == ch(']') {
                em_classe = false;
            }
            i += 1;
            continue;
        }
        if c == ch('[') {
            em_classe = true;
        } else if c == ch('(') {
            if p.get(i + 1) != Some(&ch('?')) {
                total += 1;
            } else if p.get(i + 2) == Some(&ch('<'))
                && p.get(i + 3).copied() != Some(ch('='))
                && p.get(i + 3).copied() != Some(ch('!'))
            {
                total += 1;
                let mut j = i + 3;
                let mut bruto = Vec::new();
                while j < p.len() && p[j] != ch('>') {
                    bruto.push(p[j]);
                    j += 1;
                }
                let nome = decodifica_nome(&bruto).unwrap_or(bruto);
                if nomes.iter().any(|(n, _)| *n == nome) {
                    return Err("nome de grupo duplicado".to_string());
                }
                nomes.push((nome, total));
                i = j;
            }
        }
        i += 1;
    }
    Ok((total, nomes))
}

/// Preenche o `mapa` de cada classe com a resposta de `classe_casa_lento` para 0..256.
fn preparar(no: &mut No, opc: &Flags) {
    match no {
        No::Classe(cl) => {
            let p = Programa { raiz: No::Seq(Vec::new()), grupos: 0, nomes: Vec::new(), opc: opc.clone(), prefixo: None, primeiros: None };
            for c in 0..256u32 {
                if p.classe_casa_lento(cl, c) {
                    cl.mapa[c as usize >> 6] |= 1 << (c & 63);
                }
            }
        }
        No::Grupo(n, _) | No::Rep(n, _) | No::Olhar(n, _, _) => preparar(n, opc),
        No::Alt(v) | No::Seq(v) => v.iter_mut().for_each(|n| preparar(n, opc)),
        _ => {}
    }
}

/// Junta em `ok` as unidades que podem abrir um casamento de `no`. Devolve se `no` pode casar
/// vazio; None = desiste (algo pode começar fora de 0..256, ou não dá para saber).
fn primeiros_de(no: &No, opc: &Flags, ok: &mut [u64; 4]) -> Option<bool> {
    match no {
        // Com `i`, 'k' casa também U+212A: fica sem atalho.
        No::Char(c) if *c < 256 && !opc.ignorar_caixa => {
            ok[*c as usize >> 6] |= 1 << (c & 63);
            Some(false)
        }
        // Só classes que não casam nada >= 256 (uma unidade alta pode ser o início de um par).
        No::Classe(cl) if !cl.negada && !opc.ignorar_caixa && cl.itens.iter().all(|it| match it {
            Item::Faixa(_, b) => *b < 256,
            Item::Pred(Pred::Dig, false) | Item::Pred(Pred::Pal, false) => true,
            _ => false,
        }) => {
            for (o, m) in ok.iter_mut().zip(cl.mapa) {
                *o |= m;
            }
            Some(false)
        }
        No::Inicio | No::Fim | No::Fronteira(_) | No::Olhar(..) => Some(true),
        No::Grupo(n, _) => primeiros_de(n, opc, ok),
        No::Rep(n, q) => Some(primeiros_de(n, opc, ok)? || q.min == 0),
        No::Seq(v) => {
            for n in v {
                if !primeiros_de(n, opc, ok)? {
                    return Some(false);
                }
            }
            Some(true)
        }
        No::Alt(v) => {
            let mut vazio = false;
            for n in v {
                vazio |= primeiros_de(n, opc, ok)?;
            }
            Some(vazio)
        }
        _ => None,
    }
}

/// Primeiro caractere obrigatório do padrão, para o atalho de `procurar`.
fn prefixo_literal(no: &No) -> Option<u16> {
    match no {
        No::Char(c) if *c < 0x10000 && !(0xD800..0xE000).contains(c) => Some(*c as u16),
        No::Seq(v) => v.first().and_then(prefixo_literal),
        No::Grupo(inner, _) => prefixo_literal(inner),
        _ => None,
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn prog(p: &str, fl: &str) -> Programa {
        Programa::compilar(&u(p), &Flags::de_texto(&u(fl)).unwrap()).unwrap()
    }

    fn erro(p: &str, fl: &str) -> bool {
        match Flags::de_texto(&u(fl)) {
            Ok(f) => Programa::compilar(&u(p), &f).is_err(),
            Err(_) => true,
        }
    }

    #[test]
    fn atalho_do_primeiro_caractere_nao_muda_o_resultado() {
        assert!(prog("[a-z0-9]+@x", "").primeiros.is_some());
        assert_eq!(prog("[a-z0-9]+@x", "").procurar(&u(", ab@x"), 0), Some(vec![Some((2, 6))]));
        assert_eq!(prog("(?:\\d|z)+", "").procurar(&u("..z12"), 0), Some(vec![Some((2, 5))]));
        assert_eq!(prog("(?<=\\$)\\d+", "").procurar(&u("1$23"), 0), Some(vec![Some((2, 4))]));
        // Pode casar vazio, casa fora de 0..256 ou usa `i`: sem atalho.
        assert!(prog("a*", "").primeiros.is_none());
        assert!(prog("[^a]", "").primeiros.is_none());
        assert!(prog("k", "iu").primeiros.is_none());
        assert!(prog("k", "iu").procurar(&u("\u{212A}"), 0).is_some());
        assert_eq!(prog("[a-c]+", "i").procurar(&u("xBcA"), 0), Some(vec![Some((1, 4))]));
    }

    #[test]
    fn literal_procura_no_meio() {
        assert_eq!(prog("abc", "").procurar(&u("xxabcx"), 0), Some(vec![Some((2, 5))]));
    }

    #[test]
    fn alternativa_casa_uma_das_opcoes() {
        assert!(prog("a|b", "").casar_em(&u("b"), 0).is_some());
        assert!(prog("a|b", "").casar_em(&u("c"), 0).is_none());
    }

    #[test]
    fn grupo_opcional_que_nao_participa_fica_none() {
        assert_eq!(
            prog("(a)(b)?", "").casar_em(&u("a"), 0),
            Some(vec![Some((0, 1)), Some((0, 1)), None])
        );
    }

    #[test]
    fn grupos_do_exemplo_classico() {
        assert_eq!(
            prog("(a|ab)(c|bcd)(d*)", "").casar_em(&u("abcd"), 0),
            Some(vec![Some((0, 4)), Some((0, 1)), Some((1, 4)), Some((4, 4))])
        );
    }

    #[test]
    fn grupo_nomeado() {
        let p = prog("(?<ano>\\d{4})-(?:\\d\\d)", "");
        assert_eq!(p.nomes(), &[(u("ano"), 1usize)][..]);
        assert_eq!(p.grupos(), 1);
        assert_eq!(p.casar_em(&u("2024-05"), 0), Some(vec![Some((0, 7)), Some((0, 4))]));
    }

    #[test]
    fn quantificadores_guloso_e_preguicoso() {
        assert_eq!(prog("a+", "").casar_em(&u("aaa"), 0), Some(vec![Some((0, 3))]));
        assert_eq!(prog("a+?", "").casar_em(&u("aaa"), 0), Some(vec![Some((0, 1))]));
        assert_eq!(prog("a{2,3}", "").casar_em(&u("aaaa"), 0), Some(vec![Some((0, 3))]));
        assert_eq!(prog("a{2}", "").casar_em(&u("a"), 0), None);
    }

    #[test]
    fn laco_vazio_nao_trava() {
        assert_eq!(prog("(a*)*b", "").procurar(&u("aaac"), 0), None);
    }

    #[test]
    fn retro_referencia() {
        assert!(prog("(a)\\1", "").casar_em(&u("aa"), 0).is_some());
        assert!(prog("(a)\\1", "").casar_em(&u("ab"), 0).is_none());
    }

    #[test]
    fn retro_referencia_a_grupo_que_nao_participou_casa_vazio() {
        assert!(prog("(a)?\\1b", "").casar_em(&u("b"), 0).is_some());
    }

    #[test]
    fn referencia_nomeada() {
        assert!(prog("(?<n>x)\\k<n>", "").casar_em(&u("xx"), 0).is_some());
    }

    #[test]
    fn lookahead_positivo_e_negativo() {
        assert_eq!(prog("a(?=b)", "").procurar(&u("ac ab"), 0), Some(vec![Some((3, 4))]));
        assert_eq!(prog("a(?!b)", "").procurar(&u("ab a"), 0), Some(vec![Some((3, 4))]));
    }

    #[test]
    fn lookbehind_positivo_e_negativo() {
        assert_eq!(prog("(?<=\\$)\\d+", "").procurar(&u("a$12"), 0), Some(vec![Some((2, 4))]));
        assert_eq!(prog("(?<!\\$)\\b\\d+", "").procurar(&u("$1 2"), 0), Some(vec![Some((3, 4))]));
    }

    #[test]
    fn fronteira_de_palavra() {
        assert_eq!(prog("\\bfoo\\b", "").procurar(&u("afoo foo"), 0), Some(vec![Some((5, 8))]));
    }

    #[test]
    fn multilinha_muda_ancoras() {
        assert_eq!(prog("^b", "m").procurar(&u("a\nb"), 0), Some(vec![Some((2, 3))]));
        assert_eq!(prog("^b", "").procurar(&u("a\nb"), 0), None);
        assert_eq!(prog("a$", "m").procurar(&u("a\nb"), 0), Some(vec![Some((0, 1))]));
    }

    #[test]
    fn classe_negada_e_intervalo() {
        assert!(prog("[^a-c]", "").casar_em(&u("d"), 0).is_some());
        assert!(prog("[^a-c]", "").casar_em(&u("b"), 0).is_none());
        assert!(prog("[\\d_]", "").casar_em(&u("_"), 0).is_some());
    }

    #[test]
    fn ignorar_caixa() {
        assert!(prog("abc", "i").casar_em(&u("ABC"), 0).is_some());
        assert!(prog("[a-c]+", "i").casar_em(&u("BCA"), 0).is_some());
    }

    #[test]
    fn ponto_nao_casa_terminador_sem_s() {
        assert!(prog("a.b", "").casar_em(&u("a\nb"), 0).is_none());
        assert!(prog("a.b", "s").casar_em(&u("a\nb"), 0).is_some());
    }

    #[test]
    fn modo_u_conta_par_substituto_como_um() {
        assert_eq!(prog("^.$", "u").casar_em(&u("😀"), 0), Some(vec![Some((0, 2))]));
        assert!(prog("^.$", "").casar_em(&u("😀"), 0).is_none());
        assert!(prog("\\u{1F600}", "u").casar_em(&u("😀"), 0).is_some());
    }

    #[test]
    fn propriedade_unicode() {
        assert_eq!(prog("\\p{Lu}", "u").procurar(&u("aB"), 0), Some(vec![Some((1, 2))]));
    }

    #[test]
    fn propriedades_unicode_oficiais() {
        let casa = |p: &str, t: &str| prog(p, "u").casar_em(&u(t), 0).is_some();
        assert!(casa("\\p{Script=Greek}", "α") && !casa("\\p{sc=Greek}", "a"));
        assert!(casa("\\p{scx=Hira}", "\u{30FC}") && casa("\\p{Script_Extensions=Zyyy}", "1"));
        assert!(casa("\\p{ID_Start}", "ß") && !casa("\\p{ID_Start}", "1") && casa("\\p{ID_Continue}", "1"));
        assert!(casa("\\p{Emoji}", "\u{1F600}") && casa("\\p{Any}", "\u{10FFFF}"));
        assert!(!casa("\\p{Assigned}", "\u{378}") && casa("\\p{Cn}", "\u{378}"));
        assert!(casa("\\p{Cased_Letter}", "A") && casa("\\p{LC}", "a") && !casa("\\P{Ll}", "a"));
        // Nomes exatos: sem frouxidão, sem `=` em propriedade binária, sem nomes fora da lista.
        assert!(erro("\\p{ASCII=T}", "u") && erro("\\p{Hyphen}", "u") && erro("\\p{lu}", "u"));
        assert!(erro("\\p{Script}", "u") && erro("\\p{Line_Break=Alphabetic}", "u"));
    }

    #[test]
    fn modo_v_operacoes_de_conjunto() {
        let casa = |p: &str, t: &str| prog(p, "v").casar_em(&u(t), 0).is_some();
        assert!(casa("[\\d--[0-5]]", "7") && !casa("[\\d--[0-5]]", "3"));
        assert!(casa("[\\d&&[0-5]]", "3") && !casa("[\\d&&[0-5]]", "7"));
        assert!(casa("^[[a-z]--[aeiou]]+$", "bcd") && !casa("^[[a-z]--[aeiou]]+$", "bad"));
        assert!(casa("[\\p{ASCII}--[a-z]]", "A") && !casa("[\\p{ASCII}--[a-z]]", "q"));
        assert!(casa("^[^\\d]$", "x") && !casa("^[^\\d]$", "1"));
        // `\q{..}`: a cadeia mais longa primeiro.
        assert_eq!(prog("[\\q{abc|d}]", "v").procurar(&u("xabcd"), 0), Some(vec![Some((1, 4))]));
        assert_eq!(prog("[\\q{ab}x]", "v").procurar(&u("xab"), 0), Some(vec![Some((0, 1))]));
        assert_eq!(prog("[\\q{ab}--\\q{ab}]", "v").procurar(&u("ab"), 0), None);
        // Sintaxe do modo v: operadores misturados, `-` solto e pontuadores duplos são erro.
        assert!(erro("[a&&b--c]", "v") && erro("[a-]", "v") && erro("[a&&&b]", "v") && erro("[a&&!!b]", "v"));
        assert!(erro("[^\\q{ab}]", "v") && erro("[a-c&&b-d]", "v") && erro("\\q{a}", "v"));
    }

    #[test]
    fn espacos_unicode_do_js() {
        assert_eq!(prog("\\s+", "").procurar(&u("a \u{3000}b"), 0), Some(vec![Some((1, 3))]));
    }

    #[test]
    fn flags_invalidas() {
        assert!(Flags::de_texto(&u("gg")).is_err());
        assert!(Flags::de_texto(&u("x")).is_err());
        assert!(Flags::de_texto(&u("uv")).is_err());
        assert!(Flags::de_texto(&u("dgimsuy")).is_ok());
    }

    #[test]
    fn padroes_invalidos_sao_erro() {
        assert!(erro("(", ""));
        assert!(erro("a{2,1}", ""));
        assert!(erro("*a", ""));
        assert!(erro("a{", "u"));
        assert!(erro("\\p{Nao_Existe}", "u"));
    }

    #[test]
    fn modo_sem_u_aceita_sintaxe_legada() {
        assert!(prog("a{", "").casar_em(&u("a{"), 0).is_some());
        assert!(prog("]", "").casar_em(&u("]"), 0).is_some());
    }
}
