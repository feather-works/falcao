//! Escopos (`ARQUITETURA.md`, seção 5): cada nome usado na árvore é resolvido uma vez, aqui.
//!
//! Uma passada pela árvore cria os escopos e declara os vínculos na ordem em que aparecem; as
//! referências ficam anotadas e só são resolvidas no fim, quando todas as declarações (até as
//! içadas, `var` e `function`) já são conhecidas. Resolver é subir a corrente de escopos:
//!
//! - achou numa função diferente da que usa → o vínculo é **capturado** (vai para contexto);
//! - passou por `with` ou por função com `eval` direto → a referência é **dinâmica**;
//! - não achou, ou achou no topo de um script → **global** (por nome, com cache inline).

use crate::arvore::*;
use crate::atomos::{palavras as p, Atomo, MapaRapido};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EscopoId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VinculoId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoEscopo {
    Script,
    Modulo,
    /// Parâmetros e corpo de uma função.
    Funcao,
    /// Escopo só com o nome de uma expressão de função nomeada.
    NomeFuncao,
    Bloco,
    /// `catch (e)`.
    Captura,
    /// Corpo de classe (nome da classe visível dentro).
    Classe,
    /// `for (let …)`: uma cópia por volta se capturado.
    Para,
    /// Corpo de `with`: todo nome lá dentro é dinâmico.
    Com,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoVinculo {
    Var,
    Let,
    Const,
    Classe,
    /// Declaração de função (içada com valor).
    Funcao,
    Param,
    Captura,
    Import,
    /// Nome de expressão de função (só leitura, visível dentro dela).
    NomeFuncao,
    /// `arguments` implícito.
    Arguments,
}

#[derive(Clone, Debug)]
pub struct Escopo {
    pub tipo: TipoEscopo,
    pub pai: Option<EscopoId>,
    /// Função dona (`None` = topo).
    pub funcao: Option<FuncaoId>,
    pub vinculos: Vec<VinculoId>,
    /// Algum vínculo daqui foi capturado: o escopo precisa de contexto no heap.
    pub contexto: bool,
    /// Busca por nome passa por aqui (`with`, ou função com `eval` direto).
    pub dinamico: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Vinculo {
    pub nome: Atomo,
    pub tipo: TipoVinculo,
    pub escopo: EscopoId,
    /// Usado por função interna (ou alcançável por `eval`): mora num contexto.
    pub capturado: bool,
}

/// Para onde aponta um nó `Nome`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ref {
    Vinculo(VinculoId),
    /// Global, por nome.
    Global,
    /// Busca por nome em tempo de execução (`with`/`eval`), caindo no global.
    Dinamico,
}

/// Resultado da análise de escopos de uma [`Arvore`].
#[derive(Debug, Default)]
pub struct Escopos {
    pub escopos: Vec<Escopo>,
    pub vinculos: Vec<Vinculo>,
    /// Paralelo a `Arvore::nos`: o alvo de cada `Nome` (referência ou declaração).
    refs: Vec<u32>,
    /// Escopo aberto por um nó (`Bloco`, `Para*`, `Tente`, `Escolha`, `Com`).
    pub escopo_do_no: MapaRapido<u32, EscopoId>,
    /// Escopo de parâmetros/corpo de cada função, e o do nome (expressão nomeada).
    pub escopo_da_funcao: Vec<EscopoId>,
    pub escopo_do_nome_da_funcao: MapaRapido<u32, EscopoId>,
    /// Escopo de cada classe.
    pub escopo_da_classe: Vec<EscopoId>,
    /// Funções de bloco que também ganham `var` na função (Anexo B.3.3): nó → vínculo var.
    pub anexo_b: MapaRapido<u32, VinculoId>,
    pub raiz: Option<EscopoId>,
}

const SEM_REF: u32 = u32::MAX;
const REF_GLOBAL: u32 = u32::MAX - 1;
const REF_DINAMICO: u32 = u32::MAX - 2;

impl Escopos {
    pub fn referencia(&self, no: NoId) -> Option<Ref> {
        match self.refs.get(no.0 as usize).copied().unwrap_or(SEM_REF) {
            SEM_REF => None,
            REF_GLOBAL => Some(Ref::Global),
            REF_DINAMICO => Some(Ref::Dinamico),
            v => Some(Ref::Vinculo(VinculoId(v))),
        }
    }

    pub fn escopo(&self, e: EscopoId) -> &Escopo {
        &self.escopos[e.0 as usize]
    }

    pub fn vinculo(&self, v: VinculoId) -> &Vinculo {
        &self.vinculos[v.0 as usize]
    }
}

struct Analise<'t> {
    arv: &'t Arvore,
    e: Escopos,
    mapa: MapaRapido<(u32, Atomo), VinculoId>,
    atual: EscopoId,
    /// (nó `Nome`, escopo onde aparece).
    pendentes: Vec<(NoId, EscopoId)>,
    /// Escopos onde há `eval(...)` direto.
    evals: Vec<EscopoId>,
}

/// Analisa os escopos de uma árvore inteira.
pub fn analisar_escopos(arv: &Arvore) -> Escopos {
    let mut a = Analise {
        arv,
        e: Escopos {
            refs: vec![SEM_REF; arv.nos.len()],
            escopo_da_funcao: vec![EscopoId(u32::MAX); arv.funcoes.len()],
            escopo_da_classe: vec![EscopoId(u32::MAX); arv.classes.len()],
            ..Escopos::default()
        },
        mapa: MapaRapido::default(),
        atual: EscopoId(0),
        pendentes: Vec::with_capacity(arv.nos.len() / 4),
        evals: Vec::new(),
    };
    let raiz = a.abrir(if arv.modulo { TipoEscopo::Modulo } else { TipoEscopo::Script }, None);
    a.e.raiz = Some(raiz);
    a.atual = raiz;
    a.lista(arv.corpo);
    a.resolver();
    a.e
}

impl Analise<'_> {
    fn abrir(&mut self, tipo: TipoEscopo, funcao: Option<FuncaoId>) -> EscopoId {
        let id = EscopoId(self.e.escopos.len() as u32);
        let pai = (!self.e.escopos.is_empty()).then_some(self.atual);
        self.e.escopos.push(Escopo { tipo, pai, funcao, vinculos: Vec::new(), contexto: false, dinamico: false });
        id
    }

    fn funcao_atual(&self) -> Option<FuncaoId> {
        self.e.escopos[self.atual.0 as usize].funcao
    }

    /// Escopo onde moram os `var`: o da função (ou o topo).
    fn escopo_var(&self) -> EscopoId {
        let mut e = self.atual;
        loop {
            let esc = &self.e.escopos[e.0 as usize];
            if matches!(esc.tipo, TipoEscopo::Funcao | TipoEscopo::Script | TipoEscopo::Modulo) {
                return e;
            }
            e = esc.pai.unwrap();
        }
    }

    /// Anota o alvo do nó que declara `v` (no topo de script, o alvo é o global).
    fn ligar(&mut self, no: NoId, v: VinculoId) {
        let escopo = self.e.vinculos[v.0 as usize].escopo;
        self.e.refs[no.0 as usize] = if self.e.escopos[escopo.0 as usize].tipo == TipoEscopo::Script { REF_GLOBAL } else { v.0 };
    }

    fn declarar(&mut self, escopo: EscopoId, nome: Atomo, tipo: TipoVinculo) -> VinculoId {
        if let Some(&v) = self.mapa.get(&(escopo.0, nome)) {
            // `var x` repetido, `function f` + `var f`: o mesmo vínculo (a função ganha).
            if tipo == TipoVinculo::Funcao {
                self.e.vinculos[v.0 as usize].tipo = tipo;
            }
            return v;
        }
        let v = VinculoId(self.e.vinculos.len() as u32);
        self.e.vinculos.push(Vinculo { nome, tipo, escopo, capturado: false });
        self.e.escopos[escopo.0 as usize].vinculos.push(v);
        self.mapa.insert((escopo.0, nome), v);
        v
    }

    fn lista(&mut self, l: Lista) {
        for &n in self.arv.lista(l) {
            self.visitar(n);
        }
    }

    fn opc(&mut self, n: NoId) {
        if n.existe() {
            self.visitar(n);
        }
    }

    fn chave(&mut self, c: Chave) {
        if let Chave::Calculada(e) = c {
            self.visitar(e);
        }
    }

    /// Declara os nomes de um padrão (e visita os valores padrão e chaves calculadas).
    fn padrao(&mut self, n: NoId, escopo: EscopoId, tipo: TipoVinculo) {
        match *self.arv.no(n) {
            No::Nome(a) => {
                let v = self.declarar(escopo, a, tipo);
                self.ligar(n, v);
            }
            No::PadraoArray(l) | No::PadraoObjeto(l) => {
                for &x in self.arv.lista(l) {
                    self.padrao(x, escopo, tipo);
                }
            }
            No::PadraoComValor { alvo, valor } => {
                self.padrao(alvo, escopo, tipo);
                self.visitar(valor);
            }
            No::PadraoResto(x) => self.padrao(x, escopo, tipo),
            No::Propriedade { chave, valor, .. } => {
                self.chave(chave);
                self.padrao(valor, escopo, tipo);
            }
            No::Buraco => {}
            _ => self.visitar(n),
        }
    }

    fn decl_var(&mut self, tipo: TipoVar, decls: Lista) {
        let (escopo, tv) = match tipo {
            TipoVar::Var => (self.escopo_var(), TipoVinculo::Var),
            TipoVar::Let => (self.atual, TipoVinculo::Let),
            _ => (self.atual, TipoVinculo::Const),
        };
        for &d in self.arv.lista(decls) {
            if let No::Declarador { alvo, valor } = *self.arv.no(d) {
                self.padrao(alvo, escopo, tv);
                self.opc(valor);
            }
        }
    }

    fn com_escopo(&mut self, no: NoId, tipo: TipoEscopo, f: impl FnOnce(&mut Self)) {
        let antes = self.atual;
        let e = self.abrir(tipo, self.funcao_atual());
        self.e.escopo_do_no.insert(no.0, e);
        self.atual = e;
        f(self);
        self.atual = antes;
    }

    fn visitar(&mut self, n: NoId) {
        match *self.arv.no(n) {
            No::Nome(_) => {
                self.pendentes.push((n, self.atual));
            }
            No::This | No::Super | No::Nulo | No::Verdadeiro | No::Falso | No::Numero(_) | No::BigInt(_) | No::Texto(_) | No::PedacoTemplate(_)
            | No::Regex { .. } | No::Buraco | No::NewTarget | No::ImportMeta | No::Vazio | No::Depurador | No::Continue(_) | No::Break(_) => {}
            No::Template { exprs, .. } => self.lista(exprs),
            No::TemplateMarcado { tag, quasi } => {
                self.visitar(tag);
                self.visitar(quasi);
            }
            No::Array(l) | No::Objeto(l) | No::Sequencia(l) | No::PadraoArray(l) | No::PadraoObjeto(l) => self.lista(l),
            No::Propriedade { chave, valor, .. } => {
                self.chave(chave);
                self.visitar(valor);
            }
            No::Funcao(f) => self.funcao(f),
            No::Classe(c) => self.classe(c, None),
            No::Espalhar(x) | No::Paren(x) | No::Await(x) | No::PadraoResto(x) | No::Expressao(x) | No::Lance(x) | No::CadeiaOpcional(x) => self.visitar(x),
            No::Membro { obj, .. } | No::MembroPrivado { obj, .. } => self.visitar(obj),
            No::Indice { obj, prop, .. } => {
                self.visitar(obj);
                self.visitar(prop);
            }
            No::Chamada { func, args, .. } | No::New { func, args } => {
                if matches!(self.arv.no(func), No::Nome(p::EVAL)) && matches!(self.arv.no(n), No::Chamada { .. }) {
                    self.evals.push(self.atual);
                }
                self.visitar(func);
                self.lista(args);
            }
            No::ImportDinamico { arg, opcoes } => {
                self.visitar(arg);
                self.opc(opcoes);
            }
            No::Unario { arg, .. } => self.visitar(arg),
            No::Atualizacao { alvo, .. } => self.visitar(alvo),
            No::Binario { esq, dir, .. } | No::Logico { esq, dir, .. } => {
                self.visitar(esq);
                self.visitar(dir);
            }
            No::Condicional { teste, sim, nao } | No::Se { teste, sim, nao } => {
                self.visitar(teste);
                self.visitar(sim);
                self.opc(nao);
            }
            No::Atribuicao { alvo, valor, .. } | No::PadraoComValor { alvo, valor } => {
                self.visitar(alvo);
                self.visitar(valor);
            }
            No::Yield { arg, .. } | No::Retorne(arg) => self.opc(arg),
            No::Bloco(l) => self.com_escopo(n, TipoEscopo::Bloco, |a| a.lista(l)),
            No::Var { tipo, decls } => self.decl_var(tipo, decls),
            No::Declarador { alvo, valor } => {
                self.visitar(alvo);
                self.opc(valor);
            }
            No::Para { inicio, teste, passo, corpo } => self.com_escopo(n, TipoEscopo::Para, |a| {
                a.opc(inicio);
                a.opc(teste);
                a.opc(passo);
                a.visitar(corpo);
            }),
            No::ParaEm { esq, dir, corpo } | No::ParaDe { esq, dir, corpo, .. } => {
                // O lado direito é avaliado com os nomes do `let` já em TDZ: mesmo escopo.
                self.com_escopo(n, TipoEscopo::Para, |a| {
                    a.visitar(esq);
                    a.visitar(dir);
                    a.visitar(corpo);
                })
            }
            No::Enquanto { teste, corpo } | No::FacaEnquanto { corpo, teste } => {
                self.visitar(teste);
                self.visitar(corpo);
            }
            No::Com { obj, corpo } => {
                self.visitar(obj);
                self.com_escopo(n, TipoEscopo::Com, |a| {
                    a.e.escopos[a.atual.0 as usize].dinamico = true;
                    a.visitar(corpo);
                });
            }
            No::Escolha { disc, casos } => {
                self.visitar(disc);
                self.com_escopo(n, TipoEscopo::Bloco, |a| a.lista(casos));
            }
            No::Caso { teste, corpo } => {
                self.opc(teste);
                self.lista(corpo);
            }
            No::Rotulo { corpo, .. } => self.visitar(corpo),
            No::Tente { bloco, param, captura, finalmente } => {
                self.visitar(bloco);
                if captura.existe() {
                    self.com_escopo(n, TipoEscopo::Captura, |a| {
                        if param.existe() {
                            let e = a.atual;
                            a.padrao(param, e, TipoVinculo::Captura);
                        }
                        a.visitar(captura);
                    });
                }
                self.opc(finalmente);
            }
            No::DeclFuncao(f) => {
                if let Some(nome) = self.arv.funcao(f).nome {
                    let var = self.escopo_var();
                    if var == self.atual || self.e.escopos[self.atual.0 as usize].tipo == TipoEscopo::Para {
                        let v = self.declarar(var, nome, TipoVinculo::Funcao);
                        self.ligar(n, v);
                    } else {
                        let v = self.declarar(self.atual, nome, TipoVinculo::Funcao);
                        self.ligar(n, v);
                        // Anexo B.3.3: no modo não estrito, a função de bloco também vira `var`.
                        let f_ = self.arv.funcao(f);
                        if !f_.flags.estrita && !f_.flags.assincrona && !f_.flags.gerador && !self.mapa.contains_key(&(var.0, nome)) {
                            let vv = self.declarar(var, nome, TipoVinculo::Var);
                            self.e.anexo_b.insert(n.0, vv);
                        } else if let Some(&vv) = self.mapa.get(&(var.0, nome)) {
                            if !f_.flags.estrita && matches!(self.e.vinculos[vv.0 as usize].tipo, TipoVinculo::Var | TipoVinculo::Funcao) {
                                self.e.anexo_b.insert(n.0, vv);
                            }
                        }
                    }
                }
                self.funcao(f);
            }
            No::DeclClasse(c) => {
                let nome = self.arv.classe(c).nome;
                let fora = nome.map(|a| self.declarar(self.atual, a, TipoVinculo::Classe));
                if let Some(v) = fora {
                    self.ligar(n, v);
                }
                self.classe(c, fora);
            }
            No::Import { itens, .. } => {
                for &i in self.arv.lista(itens) {
                    if let No::Importado { nome, nome_texto, local, .. } = *self.arv.no(i) {
                        // `* as ns`: o namespace é um valor fixo; o resto lê o exportado ao vivo.
                        let ns = nome.is_none() && !nome_texto.existe();
                        let tipo = if ns { TipoVinculo::Const } else { TipoVinculo::Import };
                        let v = self.declarar(self.atual, local, tipo);
                        self.ligar(i, v);
                    }
                }
            }
            No::Importado { .. } | No::Exportado { .. } => {}
            No::ExportLista { itens, fonte } => {
                // `export { a as b }` sem `from` usa o vínculo local `a`.
                if !fonte.existe() {
                    for &i in self.arv.lista(itens) {
                        self.pendentes.push((i, self.atual));
                    }
                }
            }
            No::ExportTudo { .. } => {}
            No::ExportDecl(d) => self.visitar(d),
            No::ExportPadrao(d) => match *self.arv.no(d) {
                No::DeclFuncao(f) if self.arv.funcao(f).nome.is_none() => self.funcao(f),
                No::DeclClasse(c) if self.arv.classe(c).nome.is_none() => self.classe(c, None),
                _ => self.visitar(d),
            },
        }
    }

    fn funcao(&mut self, f: FuncaoId) {
        let func = self.arv.funcao(f);
        let antes = self.atual;
        if func.flags.expressao_nomeada {
            let e = self.abrir(TipoEscopo::NomeFuncao, self.funcao_atual());
            self.atual = e;
            self.declarar(e, func.nome.unwrap(), TipoVinculo::NomeFuncao);
            self.e.escopo_do_nome_da_funcao.insert(f.0, e);
        }
        let e = self.abrir(TipoEscopo::Funcao, Some(f));
        self.atual = e;
        self.e.escopo_da_funcao[f.0 as usize] = e;
        // shortcut: `eval` direto roda como global (não vê nem cria locais), então não torna o
        // escopo dinâmico; quando o eval direto enxergar locais, marcar aqui e nomear os contextos.
        for &x in self.arv.lista(func.params) {
            self.padrao(x, e, TipoVinculo::Param);
        }
        self.lista(func.corpo);
        self.atual = antes;
    }

    fn classe(&mut self, c: ClasseId, _fora: Option<VinculoId>) {
        let classe = self.arv.classe(c);
        let antes = self.atual;
        let e = self.abrir(TipoEscopo::Classe, self.funcao_atual());
        self.atual = e;
        self.e.escopo_da_classe[c.0 as usize] = e;
        if let Some(nome) = classe.nome {
            self.declarar(e, nome, TipoVinculo::Const);
        }
        self.opc(classe.heranca);
        if let Some(k) = classe.construtor {
            self.funcao(k);
        }
        for m in &classe.membros {
            self.chave(m.chave);
            self.opc(m.valor);
        }
        self.atual = antes;
    }

    /// Resolve as referências anotadas e marca o que é capturado.
    fn resolver(&mut self) {
        for e in std::mem::take(&mut self.evals) {
            self.capturar_tudo_acima(e);
        }
        let pendentes = std::mem::take(&mut self.pendentes);
        for (no, escopo) in pendentes {
            let nome = match *self.arv.no(no) {
                No::Nome(a) => a,
                No::Exportado { local: Some(a), .. } => a,
                _ => continue,
            };
            let r = self.resolver_nome(nome, escopo);
            self.e.refs[no.0 as usize] = r;
        }
        // Topo de módulo mora no contexto: é dele que as exportações são lidas.
        if self.arv.modulo {
            let raiz = self.e.raiz.expect("escopos sem raiz");
            for vin in self.e.vinculos.iter_mut().filter(|v| v.escopo == raiz && v.tipo != TipoVinculo::Import) {
                vin.capturado = true;
            }
        }
        for v in 0..self.e.vinculos.len() {
            let vin = self.e.vinculos[v];
            if vin.capturado {
                self.e.escopos[vin.escopo.0 as usize].contexto = true;
            }
        }
    }

    fn resolver_nome(&mut self, nome: Atomo, de: EscopoId) -> u32 {
        let funcao_ref = self.e.escopos[de.0 as usize].funcao;
        let mut dinamico = false;
        let mut e = de;
        loop {
            let esc = &self.e.escopos[e.0 as usize];
            if let Some(&v) = self.mapa.get(&(e.0, nome)) {
                if matches!(esc.tipo, TipoEscopo::Script) {
                    // Topo de script: objeto global e registro léxico global, por nome.
                    return if dinamico { REF_DINAMICO } else { REF_GLOBAL };
                }
                if esc.funcao != funcao_ref || dinamico {
                    self.e.vinculos[v.0 as usize].capturado = true;
                }
                return if dinamico { REF_DINAMICO } else { v.0 };
            }
            // `arguments` implícito: nasce na primeira função não-seta da subida.
            if nome == p::ARGUMENTS && esc.tipo == TipoEscopo::Funcao {
                let f = self.arv.funcao(esc.funcao.unwrap());
                if !f.flags.seta {
                    let v = self.declarar(e, nome, TipoVinculo::Arguments);
                    if self.e.escopos[e.0 as usize].funcao != funcao_ref || dinamico {
                        self.e.vinculos[v.0 as usize].capturado = true;
                    }
                    return if dinamico { REF_DINAMICO } else { v.0 };
                }
            }
            let (esc_dinamico, esc_pai) = (esc.dinamico, esc.pai);
            if esc_dinamico {
                dinamico = true;
                // Tudo que o `eval`/`with` alcança tem de morar em contexto.
                self.capturar_tudo_acima(e);
            }
            match esc_pai {
                Some(pai) => e = pai,
                None => return if dinamico { REF_DINAMICO } else { REF_GLOBAL },
            }
        }
    }

    fn capturar_tudo_acima(&mut self, de: EscopoId) {
        let mut e = Some(de);
        while let Some(id) = e {
            let esc = &self.e.escopos[id.0 as usize];
            for &v in &esc.vinculos {
                self.e.vinculos[v.0 as usize].capturado = true;
            }
            e = esc.pai;
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::sintaxe::analisar_script;

    fn refs_de(fonte: &str) -> (Arvore, Escopos, Vec<(String, Option<Ref>)>, crate::lexico::Lexico<'_>) {
        let (arv, lex) = analisar_script(fonte).unwrap();
        let e = analisar_escopos(&arv);
        let mut v = Vec::new();
        for (i, no) in arv.nos.iter().enumerate() {
            if let No::Nome(a) = no {
                v.push((lex.atomos.texto(*a).to_string(), e.referencia(NoId(i as u32))));
            }
        }
        (arv, e, v, lex)
    }

    #[test]
    fn locais_capturas_e_globais() {
        let (_, e, refs, _) = refs_de("var g = 1; function f(a) { var b = a; let c = b; return function() { return c + g + x } }");
        let achar = |n: &str| refs.iter().filter(|(t, _)| t == n).map(|(_, r)| *r).collect::<Vec<_>>();
        assert!(achar("g").iter().all(|r| *r == Some(Ref::Global)));
        assert!(achar("x").iter().all(|r| *r == Some(Ref::Global)));
        let Some(Ref::Vinculo(c)) = achar("c")[0] else { panic!() };
        assert!(e.vinculo(c).capturado);
        let Some(Ref::Vinculo(b)) = achar("b")[0] else { panic!() };
        assert!(!e.vinculo(b).capturado);
        let Some(Ref::Vinculo(a)) = achar("a")[0] else { panic!() };
        assert_eq!(e.vinculo(a).tipo, TipoVinculo::Param);
    }

    #[test]
    fn icamento_e_dinamico() {
        let (_, _e, refs, _) = refs_de("function f() { y = 1; var y; { let z = 2; eval('z') } } function h() { with (o) { q } }");
        let y: Vec<_> = refs.iter().filter(|(t, _)| t == "y").collect();
        assert!(y.iter().all(|(_, r)| matches!(r, Some(Ref::Vinculo(_)))), "{y:?}");
        // `eval` direto ainda roda como global: não torna o escopo dinâmico.
        assert!(refs.iter().filter(|(t, _)| t == "z").all(|(_, r)| matches!(r, Some(Ref::Vinculo(_)))));
        assert_eq!(refs.iter().find(|(t, _)| t == "q").unwrap().1, Some(Ref::Dinamico));
    }

    #[test]
    fn arguments_e_setas() {
        let (_, e, refs, _) = refs_de("function f() { return () => arguments[0] }");
        let Some(Ref::Vinculo(v)) = refs.iter().find(|(t, _)| t == "arguments").unwrap().1 else { panic!() };
        assert_eq!(e.vinculo(v).tipo, TipoVinculo::Arguments);
        assert!(e.vinculo(v).capturado);
    }
}
