//! Compilador árvore + escopos → bytecode. Este arquivo tem o esqueleto: funções, onde mora
//! cada variável (registrador ou contexto), entrada e saída de escopos e saltos que
//! atravessam `finally`. Expressões, comandos e padrões estão nos outros arquivos da pasta.

mod comandos;
mod expressoes;
mod padroes;

use crate::emissor::{Arg, Emissor, Rotulo};
use crate::funcao::{Constante, Excecao, Exportacao, Flags, FuncaoBytecode, Script};
use crate::instrucoes::Op;
use falcao_sintaxe::arvore::*;
use falcao_sintaxe::atomos::{Atomo, MapaRapido};
use falcao_sintaxe::{EscopoId, Escopos, Lexico, Ref, TipoEscopo, TipoVinculo, VinculoId};

/// Erro do gerador (limites do formato).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErroCompilacao {
    pub posicao: u32,
    pub mensagem: String,
}

pub(crate) type R<T> = Result<T, ErroCompilacao>;

/// Onde mora um vínculo.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Local {
    Reg(u32),
    Ctx { escopo: EscopoId, slot: u32 },
    /// Importação de módulo (índice em `InfoModulo::importacoes`): lida ao vivo, só leitura.
    Import(u32),
}

/// Para onde um salto vai depois de desempilhar o que precisa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destino {
    Para(Rotulo),
    Retorno,
}

/// O que precisa ser desfeito ao sair de um trecho por `break`/`continue`/`return`.
pub(crate) enum Saida {
    Contexto,
    Iterador(u32),
    Finally {
        tipo: u32,
        valor: u32,
        entrada: Rotulo,
        /// Saltos que passam por este finally: (id, destino).
        saltos: Vec<(u32, Destino)>,
    },
}

/// Alvo de `break`/`continue`.
pub(crate) struct Alvo {
    pub rotulos: Vec<Atomo>,
    pub quebra: Rotulo,
    pub continua: Option<Rotulo>,
    /// `saidas.len()` na entrada.
    pub nivel: usize,
}

/// Estado da função sendo gerada.
pub(crate) struct Func {
    pub em: Emissor,
    pub constantes: Vec<Constante>,
    mapa_const: MapaRapido<(u8, u64), u32>,
    pub topo: u32,
    pub max: u32,
    pub escopo: EscopoId,
    pub contextos: u32,
    pub saidas: Vec<Saida>,
    pub alvos: Vec<Alvo>,
    pub excecoes: Vec<Excecao>,
    pub flags: Flags,
    /// Vínculos léxicos em registrador já iniciados (leitura depois não precisa de TDZ).
    pub iniciados: MapaRapido<u32, ()>,
}

pub struct Compilador<'a> {
    pub(crate) arv: &'a Arvore,
    pub(crate) esc: &'a Escopos,
    pub(crate) lex: &'a Lexico<'a>,
    pub(crate) script: Script,
    mapa_textos: MapaRapido<Vec<u16>, u32>,
    /// Funções ainda por gerar: (função da árvore, índice reservado em `script.funcoes`).
    fila: Vec<(FuncaoId, u32, Option<u32>)>,
    pub(crate) locais: MapaRapido<u32, Local>,
    /// Destinos de `?.` (a cadeia opcional mais de dentro por último).
    pub(crate) opcionais: Vec<Rotulo>,
    /// Slot no contexto da função para o nome de expressão de função capturado.
    pub(crate) f: Func,
}

impl Func {
    fn nova(escopo: EscopoId, flags: Flags) -> Func {
        Func {
            em: Emissor::default(),
            constantes: Vec::new(),
            mapa_const: MapaRapido::default(),
            topo: 0,
            max: 0,
            escopo,
            contextos: 0,
            saidas: Vec::new(),
            alvos: Vec::new(),
            excecoes: Vec::new(),
            flags,
            iniciados: MapaRapido::default(),
        }
    }
}

/// Compila um script já analisado (árvore + escopos).
pub fn compilar(arv: &Arvore, esc: &Escopos, lex: &Lexico<'_>) -> R<Script> {
    let raiz = esc.raiz.expect("escopos sem raiz");
    let flags = Flags { estrita: arv.estrito, topo: true, assincrona: arv.modulo && arv.await_topo, ..Flags::default() };
    let mut c = Compilador {
        arv,
        esc,
        lex,
        script: Script { modulo: arv.modulo, ..Script::default() },
        mapa_textos: MapaRapido::default(),
        fila: Vec::new(),
        locais: MapaRapido::default(),
        opcionais: Vec::new(),
        f: Func::nova(raiz, flags),
    };
    c.script.funcoes.push(FuncaoBytecode::default());
    c.topo(raiz)?;
    let topo = c.fechar_funcao(None, 0, arv.corpo, 0, src_len(lex));
    c.script.funcoes[0] = topo;
    while let Some((f, idx, nome)) = c.fila.pop() {
        let fb = c.funcao(f, nome)?;
        c.script.funcoes[idx as usize] = fb;
    }
    Ok(c.script)
}

fn src_len(lex: &Lexico<'_>) -> u32 {
    lex.fonte().len() as u32
}

impl<'a> Compilador<'a> {
    // ---------------------------------------------------------------- emitir

    #[inline]
    pub(crate) fn op(&mut self, op: Op, args: &[Arg]) {
        self.f.em.emitir(op, args);
    }

    #[inline]
    pub(crate) fn op0(&mut self, op: Op) {
        self.f.em.emitir(op, &[]);
    }

    pub(crate) fn rotulo(&mut self) -> Rotulo {
        self.f.em.novo_rotulo()
    }

    pub(crate) fn por(&mut self, r: Rotulo) {
        self.f.em.por(r);
    }

    pub(crate) fn salta(&mut self, op: Op, r: Rotulo) {
        self.f.em.emitir(op, &[Arg::Para(r)]);
    }

    pub(crate) fn posicao(&mut self, no: NoId) {
        let p = self.arv.posicao(no);
        self.f.em.posicao(p);
    }

    pub(crate) fn erro<T>(&self, no: NoId, msg: &str) -> R<T> {
        Err(ErroCompilacao { posicao: if no.existe() { self.arv.posicao(no) } else { 0 }, mensagem: msg.to_string() })
    }

    // ---------------------------------------------------------------- registradores

    /// Registrador temporário (liberado com [`Compilador::liberar`]).
    pub(crate) fn temp(&mut self) -> u32 {
        let r = self.f.topo;
        self.f.topo += 1;
        self.f.max = self.f.max.max(self.f.topo);
        r
    }

    /// `n` registradores seguidos.
    pub(crate) fn temps(&mut self, n: u32) -> u32 {
        let r = self.f.topo;
        self.f.topo += n;
        self.f.max = self.f.max.max(self.f.topo);
        r
    }

    /// Libera os registradores a partir de `r` (pilha).
    pub(crate) fn liberar(&mut self, r: u32) {
        debug_assert!(r <= self.f.topo);
        self.f.topo = r;
    }

    // ---------------------------------------------------------------- constantes

    fn constante(&mut self, tipo: u8, chave: u64, c: impl FnOnce() -> Constante) -> u32 {
        if let Some(&k) = self.f.mapa_const.get(&(tipo, chave)) {
            return k;
        }
        let k = self.f.constantes.len() as u32;
        self.f.constantes.push(c());
        self.f.mapa_const.insert((tipo, chave), k);
        k
    }

    pub(crate) fn texto_u16(&mut self, u: Vec<u16>) -> u32 {
        if let Some(&i) = self.mapa_textos.get(&u) {
            return i;
        }
        let i = self.script.textos.len() as u32;
        self.script.textos.push(u.clone());
        self.mapa_textos.insert(u, i);
        i
    }

    pub(crate) fn texto_atomo(&mut self, a: Atomo) -> u32 {
        let u: Vec<u16> = self.lex.atomos.texto(a).encode_utf16().collect();
        self.texto_u16(u)
    }

    pub(crate) fn unidades_de(&self, t: TextoRef) -> Vec<u16> {
        match t {
            TextoRef::Faixa(a, b) => self.lex.fonte()[a as usize..b as usize].encode_utf16().collect(),
            TextoRef::Tabela(i) => self.lex.texto(i).to_vec(),
        }
    }

    pub(crate) fn k_nome(&mut self, a: Atomo) -> u32 {
        let t = self.texto_atomo(a);
        self.constante(1, u64::from(t), || Constante::Nome(t))
    }

    pub(crate) fn k_nome_texto(&mut self, u: Vec<u16>) -> u32 {
        let t = self.texto_u16(u);
        self.constante(1, u64::from(t), || Constante::Nome(t))
    }

    pub(crate) fn k_texto(&mut self, u: Vec<u16>) -> u32 {
        let t = self.texto_u16(u);
        self.constante(2, u64::from(t), || Constante::Texto(t))
    }

    pub(crate) fn k_numero(&mut self, n: f64) -> u32 {
        self.constante(3, n.to_bits(), || Constante::Numero(n))
    }

    pub(crate) fn k_unica(&mut self, c: Constante) -> u32 {
        self.f.constantes.push(c);
        self.f.constantes.len() as u32 - 1
    }

    /// Reserva o lugar de uma função interna e põe na fila para gerar depois.
    pub(crate) fn k_funcao(&mut self, f: FuncaoId, nome: Option<u32>) -> u32 {
        let idx = self.script.funcoes.len() as u32;
        self.script.funcoes.push(FuncaoBytecode::default());
        self.fila.push((f, idx, nome));
        self.k_unica(Constante::Funcao(idx))
    }

    // ---------------------------------------------------------------- escopos

    /// O escopo empurra contexto no runtime?
    pub(crate) fn empurra(&self, e: EscopoId) -> bool {
        let esc = self.esc.escopo(e);
        match esc.tipo {
            TipoEscopo::Script => false,
            TipoEscopo::Com => true,
            _ => esc.contexto,
        }
    }

    /// Contextos entre o escopo atual e o `alvo` (0 = contexto atual).
    pub(crate) fn profundidade(&self, alvo: EscopoId) -> u32 {
        let mut e = self.f.escopo;
        let mut d = 0;
        while e != alvo {
            if self.empurra(e) {
                d += 1;
            }
            e = self.esc.escopo(e).pai.expect("escopo do vínculo fora da corrente");
        }
        d
    }

    /// Entra num escopo: empurra o contexto, se precisa, e dá lugar aos vínculos. Os léxicos
    /// começam no buraco (TDZ). Devolve o topo de registradores para [`Compilador::sair_escopo`].
    pub(crate) fn entrar_escopo(&mut self, e: EscopoId) -> u32 {
        let base = self.f.topo;
        self.f.escopo = e;
        if self.esc.escopo(e).tipo == TipoEscopo::Script {
            return base;
        }
        let empurra = self.empurra(e);
        let mut slot = 0;
        let vinculos = self.esc.escopo(e).vinculos.clone();
        for &v in &vinculos {
            if self.locais.contains_key(&v.0) {
                continue;
            }
            let vin = self.esc.vinculo(v);
            let local = if vin.capturado && empurra {
                slot += 1;
                Local::Ctx { escopo: e, slot: slot - 1 }
            } else {
                Local::Reg(self.temp())
            };
            self.locais.insert(v.0, local);
        }
        if empurra && self.esc.escopo(e).tipo != TipoEscopo::Com {
            self.op(Op::EmpurraContexto, &[Arg::U(slot)]);
            self.f.contextos += 1;
            self.f.saidas.push(Saida::Contexto);
        }
        for &v in &vinculos {
            let vin = self.esc.vinculo(v);
            if matches!(vin.tipo, TipoVinculo::Let | TipoVinculo::Const | TipoVinculo::Classe) {
                self.op0(Op::CarregaBuraco);
                self.gravar_local(v, true);
            } else if let Some(Local::Reg(_)) = self.locais.get(&v.0) {
                if !matches!(vin.tipo, TipoVinculo::Param) {
                    self.op0(Op::CarregaUndef);
                    self.gravar_local(v, true);
                }
            }
        }
        base
    }

    pub(crate) fn sair_escopo(&mut self, e: EscopoId, base: u32) {
        if self.empurra(e) && self.esc.escopo(e).tipo != TipoEscopo::Script {
            self.op0(Op::SaiContexto);
            self.f.contextos -= 1;
            self.f.saidas.pop();
        }
        self.f.escopo = self.esc.escopo(e).pai.unwrap_or(e);
        self.liberar(base);
    }

    // ---------------------------------------------------------------- nomes

    pub(crate) fn local(&self, v: VinculoId) -> Option<Local> {
        self.locais.get(&v.0).copied()
    }

    fn lexico(&self, v: VinculoId) -> bool {
        matches!(self.esc.vinculo(v).tipo, TipoVinculo::Let | TipoVinculo::Const | TipoVinculo::Classe)
    }

    /// acc = valor do vínculo `v` (com TDZ quando precisa). `nome` é para a mensagem.
    pub(crate) fn ler_vinculo(&mut self, v: VinculoId, nome: Atomo) {
        let vin = self.esc.vinculo(v);
        if vin.tipo == TipoVinculo::NomeFuncao && self.local(v).is_none() {
            self.op0(Op::CarregaFuncao);
            return;
        }
        match self.local(v) {
            Some(Local::Reg(r)) => {
                self.op(Op::Carrega, &[Arg::U(r)]);
                if self.lexico(v) && !self.f.iniciados.contains_key(&v.0) {
                    let k = self.k_nome(nome);
                    self.op(Op::ChecaTdz, &[Arg::U(k)]);
                }
            }
            Some(Local::Ctx { escopo, slot }) => {
                let d = self.profundidade(escopo);
                let op = if self.lexico(v) { Op::LeContextoChecado } else { Op::LeContexto };
                self.op(op, &[Arg::U(d), Arg::U(slot)]);
            }
            Some(Local::Import(k)) => self.op(Op::LeImport, &[Arg::U(k)]),
            // Vínculo de outra função que não foi capturado não existe; por garantia, global.
            None => {
                let k = self.k_nome(nome);
                self.op(Op::LeGlobal, &[Arg::U(k), Arg::Cache]);
            }
        }
    }

    /// vínculo = acc. `iniciar`: é a declaração (const pode, e acaba a TDZ).
    pub(crate) fn gravar_local(&mut self, v: VinculoId, _iniciar: bool) {
        match self.local(v) {
            Some(Local::Reg(r)) => self.op(Op::Guarda, &[Arg::U(r)]),
            Some(Local::Ctx { escopo, slot }) => {
                let d = self.profundidade(escopo);
                self.op(Op::GravaContexto, &[Arg::U(d), Arg::U(slot)]);
            }
            Some(Local::Import(_)) => {
                let k = self.k_texto("Assignment to constant variable.".encode_utf16().collect());
                self.op(Op::LancaErro, &[Arg::U(k)]);
            }
            None => {}
        }
    }

    /// Marca que o vínculo léxico em registrador foi iniciado (leituras depois sem TDZ).
    pub(crate) fn iniciado(&mut self, v: VinculoId) {
        self.f.iniciados.insert(v.0, ());
    }

    /// acc = valor do nome no nó `no` (um `Nome`).
    pub(crate) fn ler_nome(&mut self, no: NoId, a: Atomo, typeof_: bool) {
        match self.esc.referencia(no) {
            Some(Ref::Vinculo(v)) => self.ler_vinculo(v, a),
            Some(Ref::Dinamico) => {
                let k = self.k_nome(a);
                let op = if typeof_ { Op::LeDinamicoTypeof } else { Op::LeDinamico };
                self.op(op, &[Arg::U(k), Arg::Cache]);
            }
            Some(Ref::Global) | None => {
                if a == falcao_sintaxe::palavras::UNDEFINED {
                    self.op0(Op::CarregaUndef);
                    return;
                }
                let k = self.k_nome(a);
                let op = if typeof_ { Op::LeGlobalTypeof } else { Op::LeGlobal };
                self.op(op, &[Arg::U(k), Arg::Cache]);
            }
        }
    }

    /// nome = acc (atribuição comum: `x = 1`, `x += 1`). `declarando`: é a declaração.
    pub(crate) fn gravar_nome(&mut self, no: NoId, a: Atomo, declarando: bool) {
        match self.esc.referencia(no) {
            Some(Ref::Vinculo(v)) => {
                let vin = *self.esc.vinculo(v);
                if !declarando && vin.tipo == TipoVinculo::Const {
                    let k = self.k_nome(a);
                    // Ler antes dá ReferenceError se estiver em TDZ, como manda a norma.
                    let t = self.temp();
                    self.op(Op::Guarda, &[Arg::U(t)]);
                    self.ler_vinculo(v, a);
                    self.liberar(t);
                    self.op(Op::GravaConst, &[Arg::U(k)]);
                    return;
                }
                if vin.tipo == TipoVinculo::NomeFuncao && !declarando {
                    // Nome de expressão de função é só leitura (no estrito, TypeError).
                    if self.f.flags.estrita {
                        let k = self.k_nome(a);
                        self.op(Op::GravaConst, &[Arg::U(k)]);
                    }
                    return;
                }
                if !declarando && self.lexico(v) && !self.f.iniciados.contains_key(&v.0) {
                    // `x = 1` antes do `let x`: confere a TDZ sem perder o acc.
                    let t = self.temp();
                    self.op(Op::Guarda, &[Arg::U(t)]);
                    self.ler_vinculo(v, a);
                    self.op(Op::Carrega, &[Arg::U(t)]);
                    self.liberar(t);
                }
                self.gravar_local(v, declarando);
                if declarando {
                    self.iniciado(v);
                }
            }
            Some(Ref::Dinamico) => {
                let k = self.k_nome(a);
                self.op(Op::GravaDinamico, &[Arg::U(k), Arg::Cache]);
            }
            Some(Ref::Global) | None => {
                let k = self.k_nome(a);
                if declarando {
                    self.op(Op::IniciaGlobal, &[Arg::U(k)]);
                } else {
                    self.op(Op::GravaGlobal, &[Arg::U(k), Arg::Cache]);
                }
            }
        }
    }

    // ---------------------------------------------------------------- saltos

    /// Sai até o nível `nivel` de `saidas` e vai para `destino` (passando pelos finally).
    pub(crate) fn sair_para(&mut self, nivel: usize, destino: Destino) {
        let mut i = self.f.saidas.len();
        while i > nivel {
            i -= 1;
            match &mut self.f.saidas[i] {
                Saida::Contexto => self.op0(Op::SaiContexto),
                Saida::Iterador(r) => {
                    let r = *r;
                    self.op(Op::FechaIterador, &[Arg::U(r)]);
                }
                Saida::Finally { tipo, valor, entrada, saltos } => {
                    let id = saltos.len() as u32 + 2;
                    saltos.push((id, destino));
                    let (tipo, valor, entrada) = (*tipo, *valor, *entrada);
                    if destino == Destino::Retorno {
                        self.op(Op::Guarda, &[Arg::U(valor)]);
                    }
                    self.op(Op::CarregaInt, &[Arg::I(id as i32)]);
                    self.op(Op::Guarda, &[Arg::U(tipo)]);
                    self.salta(Op::Salta, entrada);
                    return;
                }
            }
        }
        match destino {
            Destino::Para(r) => self.salta(Op::Salta, r),
            Destino::Retorno => self.op0(Op::Retorna),
        }
    }

    // ---------------------------------------------------------------- funções

    /// Topo do script ou do módulo.
    fn topo(&mut self, raiz: EscopoId) -> R<()> {
        let corpo = self.arv.corpo;
        if self.arv.modulo {
            let namespaces = self.importacoes();
            let base = self.entrar_escopo(raiz);
            self.op0(Op::RegistraModulo);
            self.exportacoes()?;
            for (v, req) in namespaces {
                self.op(Op::CarregaNamespace, &[Arg::U(req)]);
                self.gravar_local(v, true);
                self.iniciado(v);
            }
            self.icar_funcoes(corpo)?;
            self.lista_comandos(corpo)?;
            self.sair_escopo(raiz, base);
        } else {
            self.f.escopo = raiz;
            // Declarações globais: var/function no objeto global, let/const/class no léxico.
            for &v in &self.esc.escopo(raiz).vinculos.clone() {
                let vin = *self.esc.vinculo(v);
                let k = self.k_nome(vin.nome);
                match vin.tipo {
                    TipoVinculo::Let => self.op(Op::DeclaraGlobalLexico, &[Arg::U(k), Arg::U(0)]),
                    TipoVinculo::Const | TipoVinculo::Classe => self.op(Op::DeclaraGlobalLexico, &[Arg::U(k), Arg::U(1)]),
                    TipoVinculo::Funcao => {}
                    _ => self.op(Op::DeclaraGlobal, &[Arg::U(k), Arg::U(0)]),
                }
            }
            self.icar_funcoes(corpo)?;
            // O valor do script é o do último comando de expressão (para eval).
            self.op0(Op::CarregaUndef);
            self.lista_comandos(corpo)?;
        }
        self.op0(Op::Retorna);
        Ok(())
    }

    /// Cria as funções declaradas direto nesta lista (içadas para o começo do escopo).
    pub(crate) fn icar_funcoes(&mut self, l: Lista) -> R<()> {
        for &n in self.arv.lista(l) {
            let decl = match *self.arv.no(n) {
                No::DeclFuncao(f) => Some((n, f, false)),
                No::ExportDecl(d) | No::ExportPadrao(d) => match *self.arv.no(d) {
                    No::DeclFuncao(f) => Some((d, f, matches!(self.arv.no(n), No::ExportPadrao(_)))),
                    _ => None,
                },
                _ => None,
            };
            let Some((no, f, padrao)) = decl else { continue };
            let nome = self.arv.funcao(f).nome;
            let tn = match nome {
                Some(a) => Some(self.texto_atomo(a)),
                None if padrao => Some(self.texto_u16("default".encode_utf16().collect())),
                None => None,
            };
            let k = self.k_funcao(f, tn);
            self.op(Op::CriaFuncao, &[Arg::U(k)]);
            if padrao && nome.is_none() {
                self.op0(Op::GuardaPadrao);
                continue;
            }
            match (self.esc.referencia(no), nome) {
                (Some(Ref::Vinculo(v)), _) => {
                    self.gravar_local(v, true);
                    self.iniciado(v);
                }
                (_, Some(a)) => {
                    let k = self.k_nome(a);
                    self.op(Op::DeclaraGlobal, &[Arg::U(k), Arg::U(1)]);
                }
                _ => {}
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- módulos

    /// Nome de import/export: átomo ou texto entre aspas.
    fn nome_modulo(&self, a: Option<Atomo>, texto: NoId) -> Vec<u16> {
        if let Some(a) = a {
            return self.lex.atomos.texto(a).encode_utf16().collect();
        }
        match texto.existe().then(|| self.arv.no(texto)) {
            Some(No::Texto(t)) => self.unidades_de(*t),
            _ => Vec::new(),
        }
    }

    /// Índice do especificador `fonte` (um `Texto`) em `requisitos`.
    fn requisito(&mut self, fonte: NoId) -> u32 {
        let u = self.nome_modulo(None, fonte);
        let r = &mut self.script.info_modulo.requisitos;
        match r.iter().position(|x| *x == u) {
            Some(i) => i as u32,
            None => {
                r.push(u);
                r.len() as u32 - 1
            }
        }
    }

    /// Registra os `import` (antes de dar lugar aos vínculos do topo). Devolve os `* as ns`.
    fn importacoes(&mut self) -> Vec<(VinculoId, u32)> {
        let mut ns = Vec::new();
        for &n in &self.arv.lista(self.arv.corpo).to_vec() {
            let (itens, fonte) = match *self.arv.no(n) {
                No::Import { itens, fonte } => (itens, fonte),
                No::ExportLista { fonte, .. } | No::ExportTudo { fonte, .. } if fonte.existe() => {
                    self.requisito(fonte);
                    continue;
                }
                _ => continue,
            };
            let req = self.requisito(fonte);
            for &i in self.arv.lista(itens) {
                let No::Importado { nome, nome_texto, .. } = *self.arv.no(i) else { continue };
                let Some(Ref::Vinculo(v)) = self.esc.referencia(i) else { continue };
                if nome.is_none() && !nome_texto.existe() {
                    ns.push((v, req));
                    continue;
                }
                let k = self.script.info_modulo.importacoes.len() as u32;
                let nome = self.nome_modulo(nome, nome_texto);
                self.script.info_modulo.importacoes.push((req, nome));
                self.locais.insert(v.0, Local::Import(k));
            }
        }
        ns
    }

    /// Como o vínculo `v` do topo é exportado.
    fn exportacao_de(&self, v: VinculoId, no: NoId) -> R<Exportacao> {
        match self.local(v) {
            Some(Local::Ctx { slot, .. }) => Ok(Exportacao::Local(slot)),
            Some(Local::Import(k)) => {
                let (req, nome) = &self.script.info_modulo.importacoes[k as usize];
                Ok(Exportacao::Indireta(*req, nome.clone()))
            }
            _ => self.erro(no, "exportação de vínculo fora do contexto do módulo"),
        }
    }

    /// `Nome`s ligados por um padrão de declaração.
    fn nomes_do_padrao(&self, n: NoId, saida: &mut Vec<NoId>) {
        match *self.arv.no(n) {
            No::Nome(_) => saida.push(n),
            No::PadraoArray(l) | No::PadraoObjeto(l) => {
                for &x in self.arv.lista(l) {
                    self.nomes_do_padrao(x, saida);
                }
            }
            No::Propriedade { valor: x, .. } | No::PadraoComValor { alvo: x, .. } | No::PadraoResto(x) => self.nomes_do_padrao(x, saida),
            _ => {}
        }
    }

    /// Monta `InfoModulo::exportacoes` (depois de `entrar_escopo` dar lugar aos vínculos).
    fn exportacoes(&mut self) -> R<()> {
        let mut saida: Vec<(Vec<u16>, Exportacao)> = Vec::new();
        for &n in &self.arv.lista(self.arv.corpo).to_vec() {
            match *self.arv.no(n) {
                No::ExportDecl(d) => {
                    let mut nomes = Vec::new();
                    match *self.arv.no(d) {
                        No::Var { decls, .. } => {
                            for &x in self.arv.lista(decls) {
                                if let No::Declarador { alvo, .. } = *self.arv.no(x) {
                                    self.nomes_do_padrao(alvo, &mut nomes);
                                }
                            }
                        }
                        _ => nomes.push(d),
                    }
                    for no in nomes {
                        let nome = match *self.arv.no(no) {
                            No::Nome(a) => a,
                            No::DeclFuncao(f) => self.arv.funcao(f).nome.expect("função exportada sem nome"),
                            No::DeclClasse(c) => self.arv.classe(c).nome.expect("classe exportada sem nome"),
                            _ => continue,
                        };
                        let Some(Ref::Vinculo(v)) = self.esc.referencia(no) else { return self.erro(no, "exportação sem vínculo") };
                        saida.push((self.lex.atomos.texto(nome).encode_utf16().collect(), self.exportacao_de(v, no)?));
                    }
                }
                No::ExportPadrao(d) => {
                    let e = match *self.arv.no(d) {
                        No::DeclFuncao(f) if self.arv.funcao(f).nome.is_some() => None,
                        No::DeclClasse(c) if self.arv.classe(c).nome.is_some() => None,
                        _ => Some(Exportacao::Padrao),
                    };
                    let e = match e {
                        Some(e) => e,
                        None => match self.esc.referencia(d) {
                            Some(Ref::Vinculo(v)) => self.exportacao_de(v, d)?,
                            _ => return self.erro(d, "exportação sem vínculo"),
                        },
                    };
                    saida.push(("default".encode_utf16().collect(), e));
                }
                No::ExportLista { itens, fonte } => {
                    let req = fonte.existe().then(|| self.requisito(fonte));
                    for &i in self.arv.lista(itens) {
                        let No::Exportado { local, local_texto, exportado, exportado_texto } = *self.arv.no(i) else { continue };
                        let nome = self.nome_modulo(exportado, exportado_texto);
                        let e = match req {
                            Some(r) => Exportacao::Indireta(r, self.nome_modulo(local, local_texto)),
                            None => match self.esc.referencia(i) {
                                Some(Ref::Vinculo(v)) => self.exportacao_de(v, i)?,
                                _ => return self.erro(i, "exportação de nome que não existe no módulo"),
                            },
                        };
                        saida.push((nome, e));
                    }
                }
                No::ExportTudo { como, como_texto, fonte } => {
                    let req = self.requisito(fonte);
                    if como.is_some() || como_texto.existe() {
                        saida.push((self.nome_modulo(como, como_texto), Exportacao::Namespace(req)));
                    } else {
                        self.script.info_modulo.estrelas.push(req);
                    }
                }
                _ => {}
            }
        }
        self.script.info_modulo.exportacoes = saida;
        Ok(())
    }

    /// Gera uma função interna.
    fn funcao(&mut self, id: FuncaoId, nome: Option<u32>) -> R<FuncaoBytecode> {
        let func = self.arv.funcao(id).clone();
        let fl = func.flags;
        let flags = Flags {
            estrita: fl.estrita,
            seta: fl.seta,
            metodo: fl.metodo,
            construtor_classe: fl.construtor_classe,
            classe_derivada: fl.classe_derivada,
            gerador: fl.gerador,
            assincrona: fl.assincrona,
            getter: fl.getter,
            setter: fl.setter,
            sem_construtor: !fl.construtor_classe && (fl.seta || fl.metodo || fl.gerador || fl.assincrona),
            usa_arguments: fl.usa_arguments,
            eval_direto: fl.eval_direto,
            topo: false,
        };
        let escopo = self.esc.escopo_da_funcao[id.0 as usize];
        let antes = std::mem::replace(&mut self.f, Func::nova(escopo, flags));
        let params = self.arv.lista(func.params).to_vec();
        let np = params.len() as u32;
        self.f.topo = np;
        self.f.max = np;
        // Parâmetro simples não capturado mora direto no registrador dele.
        for (i, &p) in params.iter().enumerate() {
            if let No::Nome(_) = self.arv.no(p) {
                if let Some(Ref::Vinculo(v)) = self.esc.referencia(p) {
                    if !self.esc.vinculo(v).capturado || !self.empurra(escopo) {
                        self.locais.insert(v.0, Local::Reg(i as u32));
                    }
                }
            }
        }
        // Nome de expressão de função capturado: contexto próprio entre o de fora e o da função
        // (num registrador a função interna lia o registrador dela, não o nome).
        let nome_proprio = self.esc.escopo_do_nome_da_funcao.get(&id.0).copied().filter(|&e| self.empurra(e)).map(|e| {
            let base = self.entrar_escopo(e);
            self.op0(Op::CarregaFuncao);
            self.gravar_local(self.esc.escopo(e).vinculos[0], true);
            (e, base)
        });
        let base = self.entrar_escopo(escopo);
        if fl.gerador || fl.assincrona {
            if fl.gerador {
                self.op0(Op::IniciaGerador);
            }
        }
        // `arguments`.
        if let Some(&v) = self.esc.escopo(escopo).vinculos.iter().find(|&&v| self.esc.vinculo(v).tipo == falcao_sintaxe::TipoVinculo::Arguments) {
            // shortcut: arguments nunca é mapeado aos parâmetros (mudar um não muda o outro).
            self.op(Op::CriaArguments, &[Arg::U(0)]);
            self.gravar_local(v, true);
        }
        // Parâmetros: copia para o contexto os capturados e desestrutura os padrões.
        let mut comprimento = None;
        for (i, &p) in params.iter().enumerate() {
            match *self.arv.no(p) {
                No::Nome(a) => {
                    if let Some(Ref::Vinculo(v)) = self.esc.referencia(p) {
                        if let Some(Local::Ctx { .. }) = self.local(v) {
                            self.op(Op::Carrega, &[Arg::U(i as u32)]);
                            self.gravar_local(v, true);
                        }
                    }
                    let _ = a;
                }
                No::PadraoResto(alvo) => {
                    comprimento.get_or_insert(i as u32);
                    self.op(Op::CriaResto, &[Arg::U(i as u32)]);
                    self.atribuir_padrao(alvo, true)?;
                }
                No::PadraoComValor { .. } => {
                    comprimento.get_or_insert(i as u32);
                    self.op(Op::Carrega, &[Arg::U(i as u32)]);
                    self.atribuir_padrao(p, true)?;
                }
                _ => {
                    self.op(Op::Carrega, &[Arg::U(i as u32)]);
                    self.atribuir_padrao(p, true)?;
                }
            }
        }
        self.icar_funcoes(func.corpo)?;
        let corpo = func.corpo;
        self.lista_comandos(corpo)?;
        self.sair_escopo(escopo, base);
        if let Some((e, base)) = nome_proprio {
            self.sair_escopo(e, base);
        }
        self.op0(Op::CarregaUndef);
        self.op0(Op::Retorna);
        let comprimento = comprimento.unwrap_or(np);
        let nome = match nome {
            Some(t) => Some(t),
            None => func.nome.map(|a| self.texto_atomo(a)),
        };
        let mut fb = self.fechar_funcao(nome, np, corpo, func.inicio, func.fim);
        fb.comprimento = comprimento;
        self.f = antes;
        Ok(fb)
    }

    fn fechar_funcao(&mut self, nome: Option<u32>, parametros: u32, _corpo: Lista, inicio: u32, fim: u32) -> FuncaoBytecode {
        self.f.em.terminar();
        let f = &mut self.f;
        FuncaoBytecode {
            nome,
            codigo: std::mem::take(&mut f.em.codigo),
            constantes: std::mem::take(&mut f.constantes),
            registradores: f.max.max(parametros),
            parametros,
            comprimento: parametros,
            caches: f.em.caches,
            excecoes: std::mem::take(&mut f.excecoes),
            posicoes: std::mem::take(&mut f.em.posicoes),
            flags: f.flags,
            inicio,
            fim,
        }
    }
}
