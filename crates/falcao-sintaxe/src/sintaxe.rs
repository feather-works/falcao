//! Sintaxe (ECMA-262 seções 13 a 16): símbolos → [`Arvore`]. Descida recursiva, com
//! precedência por subida para os operadores binários. Os comandos, funções, classes e
//! módulos estão em `comandos.rs`.
//!
//! - Seta `(a, b) => …`: tenta ler parâmetros; se não vier `=>`, volta atrás e lê expressão.
//! - Desestruturação `[a, b] = x`: lê como expressão e converte em padrão no lugar
//!   ([`Analisador::para_padrao`]), trocando o tipo dos nós sem copiar a lista.

use crate::arvore::*;
use crate::atomos::{palavras as p, reservada, reservada_estrita, Atomo};
use crate::lexico::{Dado, ErroLexico, Lexico, Pont, Simbolo, Tipo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErroSintaxe {
    pub posicao: u32,
    pub mensagem: String,
}

impl From<ErroLexico> for ErroSintaxe {
    fn from(e: ErroLexico) -> Self {
        Self { posicao: e.posicao, mensagem: e.mensagem.to_string() }
    }
}

pub(crate) type R<T> = Result<T, ErroSintaxe>;

/// Onde a sintaxe está (muda ao entrar em função, classe, laço...).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ctx {
    pub estrito: bool,
    pub assincrona: bool,
    pub gerador: bool,
    pub em_funcao: bool,
    /// `super.x` permitido (método de objeto/classe).
    pub super_prop: bool,
    /// `super()` permitido (construtor de classe derivada).
    pub super_chamada: bool,
    /// `new.target` permitido.
    pub new_target: bool,
    /// `in` vale como operador (falso no início de `for (... in`).
    pub sem_in: bool,
    pub em_laco: bool,
    pub em_escolha: bool,
    /// Dentro do corpo de uma classe (campos privados `#x` podem aparecer).
    pub em_classe: bool,
    /// Inicializador de campo de classe: `arguments` proibido.
    pub em_campo: bool,
    /// Topo de módulo: `await` no topo.
    pub modulo: bool,
}

pub struct Analisador<'a> {
    pub(crate) lex: Lexico<'a>,
    pub(crate) atual: Simbolo,
    /// Fim do símbolo anterior (para a posição de nós que terminam nele).
    pub(crate) fim_anterior: u32,
    pub(crate) arv: Arvore,
    /// Pilha de rascunho para montar listas aninhadas.
    pub(crate) rascunho: Vec<NoId>,
    pub(crate) ctx: Ctx,
    /// Função que está sendo lida (para as flags de `this`, `arguments`, `eval`).
    pub(crate) funcao: Option<FuncaoId>,
    /// `{ a = 1 }` lido como objeto: só vale se virar padrão. Posições pendentes.
    pub(crate) pendentes: Vec<u32>,
    pub(crate) rotulos: Vec<(Atomo, bool)>,
    /// Posições de `(` onde a tentativa de ler parâmetros de seta já falhou: sem isto,
    /// `(a = (b = (c …` aninhado custaria 2^n.
    pub(crate) nao_e_seta: crate::atomos::MapaRapido<u32, ()>,
}

/// Estado salvo para voltar atrás (tentativa de seta).
pub(crate) struct Marca {
    pos_lex: u32,
    atual: Simbolo,
    fim_anterior: u32,
    nos: usize,
    listas: usize,
    funcoes: usize,
    classes: usize,
    rascunho: usize,
    pendentes: usize,
    ctx: Ctx,
}

/// Lê um script inteiro.
pub fn analisar_script(fonte: &str) -> Result<(Arvore, Lexico<'_>), ErroSintaxe> {
    analisar(fonte, false)
}

/// Lê um módulo inteiro (estrito, com `import`/`export` e `await` no topo).
pub fn analisar_modulo(fonte: &str) -> Result<(Arvore, Lexico<'_>), ErroSintaxe> {
    analisar(fonte, true)
}

fn analisar(fonte: &str, modulo: bool) -> Result<(Arvore, Lexico<'_>), ErroSintaxe> {
    let mut a = Analisador::novo(fonte, modulo)?;
    a.arv.modulo = modulo;
    let corpo = a.programa()?;
    a.arv.corpo = corpo;
    Ok((a.arv, a.lex))
}

impl<'a> Analisador<'a> {
    pub(crate) fn novo(fonte: &'a str, modulo: bool) -> R<Self> {
        let mut lex = Lexico::novo(fonte, modulo);
        let atual = lex.proximo(crate::lexico::Objetivo::Regex)?;
        let ctx = Ctx { estrito: modulo, modulo, assincrona: modulo, ..Ctx::default() };
        let tam = fonte.len() / 6;
        let arv = Arvore {
            nos: Vec::with_capacity(tam),
            posicoes: Vec::with_capacity(tam),
            listas: Vec::with_capacity(tam / 2),
            ..Arvore::default()
        };
        Ok(Self { lex, atual, fim_anterior: 0, arv, rascunho: Vec::with_capacity(256), ctx, funcao: None, pendentes: Vec::new(), rotulos: Vec::new(), nao_e_seta: Default::default() })
    }

    // ---------------------------------------------------------------- símbolos

    #[inline]
    pub(crate) fn avancar(&mut self) -> R<()> {
        self.fim_anterior = self.atual.fim;
        self.atual = self.lex.proximo(crate::lexico::Objetivo::Divisao)?;
        Ok(())
    }

    /// O símbolo seguinte, sem consumir.
    pub(crate) fn espiar(&mut self) -> R<Simbolo> {
        let pos = self.lex.posicao();
        let s = self.lex.proximo(crate::lexico::Objetivo::Divisao);
        self.lex.ir_para(pos);
        Ok(s?)
    }

    #[inline]
    pub(crate) fn e(&self, pt: Pont) -> bool {
        self.atual.tipo == Tipo::Pont(pt)
    }

    #[inline]
    pub(crate) fn comer(&mut self, pt: Pont) -> R<bool> {
        if self.e(pt) {
            self.avancar()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn esperar(&mut self, pt: Pont) -> R<()> {
        if self.e(pt) {
            self.avancar()
        } else {
            self.erro_aqui(&format!("esperava {}", texto_pont(pt)))
        }
    }

    /// Palavra (não escapada) no símbolo atual.
    #[inline]
    pub(crate) fn palavra(&self, a: Atomo) -> bool {
        self.atual.tipo == Tipo::Nome && self.atual.dado == Dado::Atomo(a) && !self.atual.escapado
    }

    pub(crate) fn comer_palavra(&mut self, a: Atomo) -> R<bool> {
        if self.palavra(a) {
            self.avancar()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn esperar_palavra(&mut self, a: Atomo) -> R<()> {
        if self.comer_palavra(a)? {
            Ok(())
        } else {
            let t = self.lex.atomos.texto(a).to_string();
            self.erro_aqui(&format!("esperava {t}"))
        }
    }

    #[inline]
    pub(crate) fn atomo(&self) -> Option<Atomo> {
        match (self.atual.tipo, self.atual.dado) {
            (Tipo::Nome, Dado::Atomo(a)) => Some(a),
            _ => None,
        }
    }

    pub(crate) fn erro<T>(&self, posicao: u32, msg: &str) -> R<T> {
        Err(ErroSintaxe { posicao, mensagem: msg.to_string() })
    }

    pub(crate) fn erro_aqui<T>(&self, msg: &str) -> R<T> {
        let achou = match self.atual.tipo {
            Tipo::Fim => "fim do código".to_string(),
            _ => format!("'{}'", &self.lex.fonte()[self.atual.inicio as usize..self.atual.fim as usize]),
        };
        self.erro(self.atual.inicio, &format!("{msg}, achou {achou}"))
    }

    /// Fim de comando com inserção automática de `;` (§12.10).
    pub(crate) fn fim_comando(&mut self) -> R<()> {
        if self.comer(Pont::PontoVirgula)? {
            return Ok(());
        }
        if self.e(Pont::FechaChave) || self.atual.tipo == Tipo::Fim || self.atual.quebra_antes {
            return Ok(());
        }
        self.erro_aqui("esperava ;")
    }

    pub(crate) fn marca(&self) -> Marca {
        Marca {
            pos_lex: self.lex.posicao(),
            atual: self.atual,
            fim_anterior: self.fim_anterior,
            nos: self.arv.nos.len(),
            listas: self.arv.listas.len(),
            funcoes: self.arv.funcoes.len(),
            classes: self.arv.classes.len(),
            rascunho: self.rascunho.len(),
            pendentes: self.pendentes.len(),
            ctx: self.ctx,
        }
    }

    pub(crate) fn voltar(&mut self, m: Marca) {
        self.lex.ir_para(m.pos_lex);
        self.atual = m.atual;
        self.fim_anterior = m.fim_anterior;
        self.arv.nos.truncate(m.nos);
        self.arv.posicoes.truncate(m.nos);
        self.arv.listas.truncate(m.listas);
        self.arv.funcoes.truncate(m.funcoes);
        self.arv.classes.truncate(m.classes);
        self.rascunho.truncate(m.rascunho);
        self.pendentes.truncate(m.pendentes);
        self.ctx = m.ctx;
    }

    // ---------------------------------------------------------------- arena

    #[inline]
    pub(crate) fn no(&mut self, no: No, pos: u32) -> NoId {
        let id = NoId(self.arv.nos.len() as u32);
        self.arv.nos.push(no);
        self.arv.posicoes.push(pos);
        id
    }

    /// Fecha os itens do rascunho desde `base` numa lista.
    #[inline]
    pub(crate) fn lista(&mut self, base: usize) -> Lista {
        let inicio = self.arv.listas.len() as u32;
        let len = (self.rascunho.len() - base) as u32;
        self.arv.listas.extend_from_slice(&self.rascunho[base..]);
        self.rascunho.truncate(base);
        Lista { inicio, len }
    }

    pub(crate) fn texto_ref(s: &Simbolo) -> TextoRef {
        match s.dado {
            Dado::Faixa(a, b) => TextoRef::Faixa(a, b),
            Dado::Texto(i) => TextoRef::Tabela(i),
            _ => TextoRef::Faixa(0, 0),
        }
    }

    pub(crate) fn marcar_funcao(&mut self, f: impl Fn(&mut FlagsFuncao)) {
        // `this`/`arguments` de seta pertencem à função de fora: marca até a 1ª não-seta.
        let mut atual = self.funcao;
        while let Some(id) = atual {
            let func = &mut self.arv.funcoes[id.0 as usize];
            f(&mut func.flags);
            if !func.flags.seta {
                break;
            }
            atual = func.pai;
        }
    }

    // ---------------------------------------------------------------- nomes

    /// Nome usado como identificador (referência, variável, parâmetro).
    pub(crate) fn nome_ident(&mut self) -> R<Atomo> {
        let Some(a) = self.atomo() else { return self.erro_aqui("esperava um nome") };
        self.checar_ident(a, self.atual.inicio)?;
        self.avancar()?;
        Ok(a)
    }

    pub(crate) fn checar_ident(&self, a: Atomo, pos: u32) -> R<()> {
        if reservada(a) {
            return self.erro(pos, &format!("'{}' é palavra reservada", self.lex.atomos.texto(a)));
        }
        if self.ctx.estrito && reservada_estrita(a) {
            return self.erro(pos, &format!("'{}' é reservada no modo estrito", self.lex.atomos.texto(a)));
        }
        if a == p::YIELD && self.ctx.gerador {
            return self.erro(pos, "'yield' não pode ser nome dentro de gerador");
        }
        if a == p::AWAIT && (self.ctx.assincrona || self.ctx.modulo) {
            return self.erro(pos, "'await' não pode ser nome aqui");
        }
        if a == p::ARGUMENTS && self.ctx.em_campo {
            return self.erro(pos, "'arguments' em inicializador de campo");
        }
        Ok(())
    }

    /// Nome de variável sendo declarada ou atribuída (no estrito, não `eval`/`arguments`).
    pub(crate) fn checar_vinculo(&self, a: Atomo, pos: u32) -> R<()> {
        self.checar_ident(a, pos)?;
        if self.ctx.estrito && (a == p::EVAL || a == p::ARGUMENTS) {
            return self.erro(pos, "não pode atribuir a eval/arguments no modo estrito");
        }
        Ok(())
    }

    // ---------------------------------------------------------------- expressões

    /// `Expression` (com vírgula).
    pub(crate) fn expressao(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        let primeira = self.atribuicao()?;
        if !self.e(Pont::Virgula) {
            return Ok(primeira);
        }
        let base = self.rascunho.len();
        self.rascunho.push(primeira);
        while self.comer(Pont::Virgula)? {
            let e = self.atribuicao()?;
            self.rascunho.push(e);
        }
        let l = self.lista(base);
        Ok(self.no(No::Sequencia(l), pos))
    }

    /// Expressão completa num lugar onde não pode virar padrão: confere os `{a = 1}` soltos.
    pub(crate) fn expressao_fechada(&mut self) -> R<NoId> {
        let antes = self.pendentes.len();
        let e = self.expressao()?;
        self.conferir_pendentes(antes)?;
        Ok(e)
    }

    pub(crate) fn atribuicao_fechada(&mut self) -> R<NoId> {
        let antes = self.pendentes.len();
        let e = self.atribuicao()?;
        self.conferir_pendentes(antes)?;
        Ok(e)
    }

    pub(crate) fn conferir_pendentes(&mut self, antes: usize) -> R<()> {
        if self.pendentes.len() > antes {
            let pos = self.pendentes[antes];
            return self.erro(pos, "'=' em objeto só vale em desestruturação");
        }
        Ok(())
    }

    /// `AssignmentExpression`.
    pub(crate) fn atribuicao(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        // yield
        if self.ctx.gerador && self.palavra(p::YIELD) {
            return self.yield_();
        }
        // Setas: `x =>`, `async x =>`, `(…) =>`, `async (…) =>`.
        if let Some(seta) = self.tentar_seta()? {
            return Ok(seta);
        }
        let antes = self.pendentes.len();
        let esq = self.condicional()?;
        if self.e(Pont::Seta) {
            if let No::Nome(a) = *self.arv.no(esq) {
                if self.atual.quebra_antes {
                    return self.erro_aqui("quebra de linha antes de =>");
                }
                self.checar_vinculo(a, pos)?;
                return self.seta(pos, false, ParamsSeta::Um(esq));
            }
        }
        let op = match self.atual.tipo {
            Tipo::Pont(pt) => op_atribuicao(pt),
            _ => None,
        };
        let Some(op) = op else {
            // Sem `=`: se não pode virar padrão por quem está fora, `{a=1}` aqui é erro.
            if !matches!(self.arv.no(esq), No::Array(_) | No::Objeto(_)) {
                self.conferir_pendentes(antes)?;
            }
            return Ok(esq);
        };
        if op == OpAtrib::Simples && matches!(self.arv.no(esq), No::Array(_) | No::Objeto(_)) {
            self.para_padrao(esq, false)?;
            self.pendentes.truncate(antes);
        } else {
            self.conferir_pendentes(antes)?;
            self.alvo_simples(esq)?;
        }
        self.avancar()?;
        let valor = self.atribuicao()?;
        Ok(self.no(No::Atribuicao { op, alvo: esq, valor }, pos))
    }

    /// Alvo de `+=`, `++` etc.: nome ou membro (com parênteses em volta, ainda vale).
    pub(crate) fn alvo_simples(&mut self, id: NoId) -> R<()> {
        match *self.arv.no(id) {
            No::Nome(a) => self.checar_vinculo(a, self.arv.posicao(id)),
            No::Membro { opcional: false, .. } | No::Indice { opcional: false, .. } | No::MembroPrivado { opcional: false, .. } => Ok(()),
            // Anexo B: `f() = 1` é erro só na hora de rodar (sites antigos).
            No::Chamada { opcional: false, .. } if !self.ctx.estrito => Ok(()),
            No::Paren(dentro) => self.alvo_simples(dentro),
            _ => self.erro(self.arv.posicao(id), "alvo de atribuição inválido"),
        }
    }

    /// Converte uma expressão (lida como array/objeto/nome) em padrão de atribuição, no lugar.
    /// `declaracao`: em `let/const/var`/parâmetros só nomes valem como folhas.
    pub(crate) fn para_padrao(&mut self, id: NoId, declaracao: bool) -> R<()> {
        let pos = self.arv.posicao(id);
        match *self.arv.no(id) {
            No::Nome(a) => self.checar_vinculo(a, pos),
            No::Membro { .. } | No::Indice { .. } | No::MembroPrivado { .. } if !declaracao => self.alvo_simples(id),
            No::Paren(dentro) if !declaracao && matches!(self.arv.no(dentro), No::Nome(_) | No::Membro { .. } | No::Indice { .. } | No::MembroPrivado { .. } | No::Paren(_)) => self.alvo_simples(dentro),
            No::Array(itens) => {
                let n = itens.len as usize;
                for i in 0..n {
                    let item = self.arv.listas[itens.inicio as usize + i];
                    match *self.arv.no(item) {
                        No::Buraco => {}
                        No::Espalhar(x) => {
                            if i + 1 != n {
                                return self.erro(self.arv.posicao(item), "o resto (...) tem de ser o último");
                            }
                            self.para_padrao(x, declaracao)?;
                            self.arv.nos[item.0 as usize] = No::PadraoResto(x);
                        }
                        _ => self.item_padrao(item, declaracao)?,
                    }
                }
                self.arv.nos[id.0 as usize] = No::PadraoArray(itens);
                Ok(())
            }
            No::Objeto(props) => {
                let n = props.len as usize;
                for i in 0..n {
                    let item = self.arv.listas[props.inicio as usize + i];
                    match *self.arv.no(item) {
                        No::Propriedade { valor, tipo: TipoProp::Normal | TipoProp::Curta, .. } => self.item_padrao(valor, declaracao)?,
                        No::Espalhar(x) => {
                            if i + 1 != n {
                                return self.erro(self.arv.posicao(item), "o resto (...) tem de ser o último");
                            }
                            if !matches!(self.arv.no(x), No::Nome(_)) && (declaracao || !matches!(self.arv.no(x), No::Membro { .. } | No::Indice { .. })) {
                                return self.erro(self.arv.posicao(x), "resto de objeto só aceita nome");
                            }
                            self.para_padrao(x, declaracao)?;
                            self.arv.nos[item.0 as usize] = No::PadraoResto(x);
                        }
                        _ => return self.erro(self.arv.posicao(item), "método não vale em desestruturação"),
                    }
                }
                self.arv.nos[id.0 as usize] = No::PadraoObjeto(props);
                Ok(())
            }
            No::PadraoArray(_) | No::PadraoObjeto(_) => Ok(()),
            _ => self.erro(pos, "alvo de desestruturação inválido"),
        }
    }

    /// Item de padrão: `alvo` ou `alvo = valor`.
    fn item_padrao(&mut self, item: NoId, declaracao: bool) -> R<()> {
        match *self.arv.no(item) {
            No::Atribuicao { op: OpAtrib::Simples, alvo, valor } => {
                self.para_padrao(alvo, declaracao)?;
                self.arv.nos[item.0 as usize] = No::PadraoComValor { alvo, valor };
                Ok(())
            }
            No::PadraoComValor { alvo, .. } => self.para_padrao(alvo, declaracao),
            _ => self.para_padrao(item, declaracao),
        }
    }

    fn yield_(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        if self.ctx.em_campo {
            return self.erro_aqui("'yield' em parâmetro");
        }
        self.avancar()?;
        let delegado = !self.atual.quebra_antes && self.comer(Pont::Vezes)?;
        let tem_arg = delegado
            || !(self.atual.quebra_antes
                || matches!(
                    self.atual.tipo,
                    Tipo::Fim
                        | Tipo::Pont(
                            Pont::FechaParen | Pont::FechaColchete | Pont::FechaChave | Pont::Virgula | Pont::PontoVirgula | Pont::DoisPontos | Pont::Interrogacao
                        )
                ));
        let arg = if tem_arg { self.atribuicao()? } else { NoId::NADA };
        Ok(self.no(No::Yield { arg, delegado }, pos))
    }

    /// `teste ? sim : nao`.
    fn condicional(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        let teste = self.binaria(0)?;
        if !self.e(Pont::Interrogacao) {
            return Ok(teste);
        }
        self.avancar()?;
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let sim = self.atribuicao_fechada()?;
        self.ctx.sem_in = sem_in;
        self.esperar(Pont::DoisPontos)?;
        let nao = self.atribuicao_fechada()?;
        Ok(self.no(No::Condicional { teste, sim, nao }, pos))
    }

    /// Operador binário no símbolo atual e a precedência dele (maior = mais forte).
    fn op_binario(&self) -> Option<(Op, u8)> {
        Some(match self.atual.tipo {
            Tipo::Pont(pt) => match pt {
                Pont::Coalesce => (Op::Log(OpLogico::Coalesce), 1),
                Pont::OuOu => (Op::Log(OpLogico::Ou), 2),
                Pont::EE => (Op::Log(OpLogico::E), 3),
                Pont::Ou => (Op::Bin(OpBinario::Ou), 4),
                Pont::OuExclusivo => (Op::Bin(OpBinario::OuExclusivo), 5),
                Pont::E => (Op::Bin(OpBinario::E), 6),
                Pont::Igual2 => (Op::Bin(OpBinario::Igual), 7),
                Pont::Diferente => (Op::Bin(OpBinario::Diferente), 7),
                Pont::Igual3 => (Op::Bin(OpBinario::Identico), 7),
                Pont::Diferente2 => (Op::Bin(OpBinario::NaoIdentico), 7),
                Pont::Menor => (Op::Bin(OpBinario::Menor), 8),
                Pont::Maior => (Op::Bin(OpBinario::Maior), 8),
                Pont::MenorIgual => (Op::Bin(OpBinario::MenorIgual), 8),
                Pont::MaiorIgual => (Op::Bin(OpBinario::MaiorIgual), 8),
                Pont::DeslEsq => (Op::Bin(OpBinario::DeslEsq), 9),
                Pont::DeslDir => (Op::Bin(OpBinario::DeslDir), 9),
                Pont::DeslDirSemSinal => (Op::Bin(OpBinario::DeslDirSemSinal), 9),
                Pont::Mais => (Op::Bin(OpBinario::Soma), 10),
                Pont::Menos => (Op::Bin(OpBinario::Sub), 10),
                Pont::Vezes => (Op::Bin(OpBinario::Mul), 11),
                Pont::Barra => (Op::Bin(OpBinario::Div), 11),
                Pont::Resto => (Op::Bin(OpBinario::Resto), 11),
                Pont::Potencia => (Op::Bin(OpBinario::Potencia), 12),
                _ => return None,
            },
            Tipo::Nome if self.palavra(p::INSTANCEOF) => (Op::Bin(OpBinario::Instanceof), 8),
            Tipo::Nome if self.palavra(p::IN) && !self.ctx.sem_in => (Op::Bin(OpBinario::In), 8),
            _ => return None,
        })
    }

    /// Subida de precedência: lê operandos e operadores mais fortes que `minimo`.
    fn binaria(&mut self, minimo: u8) -> R<NoId> {
        let pos = self.atual.inicio;
        // `#x in obj`.
        let mut esq = if self.atual.tipo == Tipo::NomePrivado {
            let Dado::Atomo(a) = self.atual.dado else { unreachable!() };
            self.avancar()?;
            if !self.palavra(p::IN) || self.ctx.sem_in || minimo > 8 {
                return self.erro(pos, "#nome solto só vale antes de 'in'");
            }
            self.avancar()?;
            let dir = self.binaria(9)?;
            let nome = self.no(No::Nome(a), pos);
            self.no(No::Binario { op: OpBinario::InPrivado, esq: nome, dir }, pos)
        } else {
            self.unaria()?
        };
        let mut misturou: Option<OpLogico> = None;
        while let Some((op, prec)) = self.op_binario() {
            if prec <= minimo {
                break;
            }
            // `**` é associativo à direita e não aceita unário sem parênteses à esquerda.
            if prec == 12 {
                if matches!(self.arv.no(esq), No::Unario { .. } | No::Await(_)) && self.arv.posicao(esq) == pos {
                    return self.erro_aqui("use parênteses: -a ** b é ambíguo");
                }
            }
            // `??` não mistura com `&&`/`||` sem parênteses.
            if let Op::Log(l) = op {
                match (misturou, l) {
                    (Some(OpLogico::Coalesce), OpLogico::E | OpLogico::Ou) | (Some(OpLogico::E | OpLogico::Ou), OpLogico::Coalesce) => {
                        return self.erro_aqui("?? não pode se misturar com && ou || sem parênteses");
                    }
                    _ => {}
                }
                misturou = Some(l);
            }
            self.avancar()?;
            let dir = if prec == 12 { self.binaria(11)? } else { self.binaria(prec)? };
            if let (Op::Log(l), No::Logico { op: interno, .. }) = (op, self.arv.no(dir)) {
                if (l == OpLogico::Coalesce) != (*interno == OpLogico::Coalesce) {
                    return self.erro(self.arv.posicao(dir), "?? não pode se misturar com && ou || sem parênteses");
                }
            }
            esq = match op {
                Op::Bin(b) => self.no(No::Binario { op: b, esq, dir }, pos),
                Op::Log(l) => self.no(No::Logico { op: l, esq, dir }, pos),
            };
        }
        Ok(esq)
    }

    fn unaria(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        let op = match self.atual.tipo {
            Tipo::Pont(Pont::Menos) => Some(OpUnario::Menos),
            Tipo::Pont(Pont::Mais) => Some(OpUnario::Mais),
            Tipo::Pont(Pont::Nao) => Some(OpUnario::Nao),
            Tipo::Pont(Pont::Til) => Some(OpUnario::Til),
            Tipo::Nome if self.palavra(p::TYPEOF) => Some(OpUnario::Typeof),
            Tipo::Nome if self.palavra(p::VOID) => Some(OpUnario::Void),
            Tipo::Nome if self.palavra(p::DELETE) => Some(OpUnario::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.avancar()?;
            let arg = self.unaria()?;
            if op == OpUnario::Delete && self.ctx.estrito && matches!(self.arv.no(arg), No::Nome(_)) {
                return self.erro(pos, "delete de nome no modo estrito");
            }
            if op == OpUnario::Delete && matches!(self.arv.no(arg), No::MembroPrivado { .. }) {
                return self.erro(pos, "delete de campo privado");
            }
            return Ok(self.no(No::Unario { op, arg }, pos));
        }
        if self.e(Pont::MaisMais) || self.e(Pont::MenosMenos) {
            let incremento = self.e(Pont::MaisMais);
            self.avancar()?;
            let alvo = self.unaria()?;
            self.alvo_simples(alvo)?;
            return Ok(self.no(No::Atualizacao { incremento, prefixo: true, alvo }, pos));
        }
        if self.palavra(p::AWAIT) && (self.ctx.assincrona || (self.ctx.modulo && !self.ctx.em_funcao)) {
            if self.ctx.em_campo {
                return self.erro_aqui("'await' em parâmetro");
            }
            self.arv.await_topo |= !self.ctx.em_funcao;
            self.avancar()?;
            let arg = self.unaria()?;
            return Ok(self.no(No::Await(arg), pos));
        }
        let e = self.lhs()?;
        // Pós-fixo: sem quebra de linha antes de `++`/`--`.
        if (self.e(Pont::MaisMais) || self.e(Pont::MenosMenos)) && !self.atual.quebra_antes {
            let incremento = self.e(Pont::MaisMais);
            self.alvo_simples(e)?;
            self.avancar()?;
            return Ok(self.no(No::Atualizacao { incremento, prefixo: false, alvo: e }, pos));
        }
        Ok(e)
    }

    /// `LeftHandSideExpression`: `new`, membros, chamadas, `?.`, templates marcados.
    pub(crate) fn lhs(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        let mut e = if self.palavra(p::NEW) { self.new_()? } else { self.primaria()? };
        let mut opcional = false;
        loop {
            match self.atual.tipo {
                Tipo::Pont(Pont::Ponto) => {
                    self.avancar()?;
                    e = self.membro_depois_do_ponto(e, false, pos)?;
                }
                Tipo::Pont(Pont::InterrogacaoPonto) => {
                    self.avancar()?;
                    opcional = true;
                    e = match self.atual.tipo {
                        Tipo::Pont(Pont::AbreParen) => {
                            let args = self.argumentos()?;
                            self.no(No::Chamada { func: e, args, opcional: true }, pos)
                        }
                        Tipo::Pont(Pont::AbreColchete) => {
                            self.avancar()?;
                            let prop = self.expressao_dentro()?;
                            self.esperar(Pont::FechaColchete)?;
                            self.no(No::Indice { obj: e, prop, opcional: true }, pos)
                        }
                        Tipo::Template(_) => return self.erro_aqui("template marcado em cadeia opcional"),
                        _ => self.membro_depois_do_ponto(e, true, pos)?,
                    };
                }
                Tipo::Pont(Pont::AbreColchete) => {
                    self.avancar()?;
                    let prop = self.expressao_dentro()?;
                    self.esperar(Pont::FechaColchete)?;
                    e = self.no(No::Indice { obj: e, prop, opcional: false }, pos);
                }
                Tipo::Pont(Pont::AbreParen) => {
                    let args = self.argumentos()?;
                    if let No::Nome(a) = self.arv.no(e) {
                        if *a == p::EVAL {
                            self.marcar_funcao(|f| f.eval_direto = true);
                        }
                    }
                    e = self.no(No::Chamada { func: e, args, opcional: false }, pos);
                }
                Tipo::Template(_) => {
                    if opcional {
                        return self.erro_aqui("template marcado em cadeia opcional");
                    }
                    let quasi = self.template(true)?;
                    e = self.no(No::TemplateMarcado { tag: e, quasi }, pos);
                }
                _ => break,
            }
        }
        if opcional {
            e = self.no(No::CadeiaOpcional(e), pos);
        }
        Ok(e)
    }

    /// Expressão completa dentro de `[...]`/`(...)` (o `in` volta a valer).
    pub(crate) fn expressao_dentro(&mut self) -> R<NoId> {
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let e = self.expressao_fechada();
        self.ctx.sem_in = sem_in;
        e
    }

    fn membro_depois_do_ponto(&mut self, obj: NoId, opcional: bool, pos: u32) -> R<NoId> {
        match (self.atual.tipo, self.atual.dado) {
            (Tipo::Nome, Dado::Atomo(prop)) => {
                self.avancar()?;
                Ok(self.no(No::Membro { obj, prop, opcional }, pos))
            }
            (Tipo::NomePrivado, Dado::Atomo(prop)) => {
                if !self.ctx.em_classe {
                    return self.erro_aqui("#nome fora de classe");
                }
                self.avancar()?;
                Ok(self.no(No::MembroPrivado { obj, prop, opcional }, pos))
            }
            _ => self.erro_aqui("esperava nome de propriedade depois do ponto"),
        }
    }

    pub(crate) fn argumentos(&mut self) -> R<Lista> {
        self.esperar(Pont::AbreParen)?;
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let base = self.rascunho.len();
        while !self.e(Pont::FechaParen) {
            let pos = self.atual.inicio;
            let item = if self.comer(Pont::Reticencias)? {
                let x = self.atribuicao_fechada()?;
                self.no(No::Espalhar(x), pos)
            } else {
                self.atribuicao_fechada()?
            };
            self.rascunho.push(item);
            if !self.comer(Pont::Virgula)? {
                break;
            }
        }
        self.ctx.sem_in = sem_in;
        self.esperar(Pont::FechaParen)?;
        Ok(self.lista(base))
    }

    fn new_(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        if self.comer(Pont::Ponto)? {
            if !self.palavra(p::TARGET) {
                return self.erro_aqui("esperava new.target");
            }
            if !self.ctx.new_target {
                return self.erro_aqui("new.target fora de função");
            }
            self.avancar()?;
            return Ok(self.no(No::NewTarget, pos));
        }
        // O alvo: `new` aninhado ou primária, com membros (sem chamadas).
        let mut func = if self.palavra(p::NEW) { self.new_()? } else { self.primaria()? };
        if matches!(self.arv.no(func), No::ImportDinamico { .. }) {
            return self.erro(pos, "new import(...) não vale");
        }
        loop {
            match self.atual.tipo {
                Tipo::Pont(Pont::Ponto) => {
                    self.avancar()?;
                    func = self.membro_depois_do_ponto(func, false, pos)?;
                }
                Tipo::Pont(Pont::AbreColchete) => {
                    self.avancar()?;
                    let prop = self.expressao_dentro()?;
                    self.esperar(Pont::FechaColchete)?;
                    func = self.no(No::Indice { obj: func, prop, opcional: false }, pos);
                }
                Tipo::Template(_) => {
                    let quasi = self.template(true)?;
                    func = self.no(No::TemplateMarcado { tag: func, quasi }, pos);
                }
                Tipo::Pont(Pont::InterrogacaoPonto) => return self.erro_aqui("?. depois de new"),
                _ => break,
            }
        }
        let args = if self.e(Pont::AbreParen) { self.argumentos()? } else { Lista::default() };
        Ok(self.no(No::New { func, args }, pos))
    }

    fn primaria(&mut self) -> R<NoId> {
        let s = self.atual;
        let pos = s.inicio;
        match s.tipo {
            Tipo::Nome => {
                let Dado::Atomo(a) = s.dado else { unreachable!() };
                if !s.escapado {
                    match a {
                        p::THIS => {
                            self.avancar()?;
                            self.marcar_funcao(|f| f.usa_this = true);
                            return Ok(self.no(No::This, pos));
                        }
                        p::NULL => {
                            self.avancar()?;
                            return Ok(self.no(No::Nulo, pos));
                        }
                        p::TRUE => {
                            self.avancar()?;
                            return Ok(self.no(No::Verdadeiro, pos));
                        }
                        p::FALSE => {
                            self.avancar()?;
                            return Ok(self.no(No::Falso, pos));
                        }
                        p::FUNCTION => return self.expressao_funcao(false, pos),
                        p::CLASS => {
                            let c = self.classe(false)?;
                            return Ok(self.no(No::Classe(c), pos));
                        }
                        p::SUPER => return self.super_(),
                        p::IMPORT => return self.import_expr(),
                        p::NEW => return self.new_(),
                        p::ASYNC => {
                            let prox = self.espiar()?;
                            if prox.tipo == Tipo::Nome && prox.dado == Dado::Atomo(p::FUNCTION) && !prox.quebra_antes && !prox.escapado {
                                self.avancar()?;
                                return self.expressao_funcao(true, pos);
                            }
                        }
                        _ => {}
                    }
                }
                let a = self.nome_ident()?;
                if a == p::ARGUMENTS {
                    self.marcar_funcao(|f| f.usa_arguments = true);
                }
                Ok(self.no(No::Nome(a), pos))
            }
            Tipo::Numero => {
                if s.escapado && self.ctx.estrito {
                    return self.erro_aqui("número octal legado no modo estrito");
                }
                let Dado::Numero(n) = s.dado else { unreachable!() };
                self.avancar()?;
                Ok(self.no(No::Numero(n), pos))
            }
            Tipo::BigInt => {
                let Dado::Texto(i) = s.dado else { unreachable!() };
                self.avancar()?;
                Ok(self.no(No::BigInt(i), pos))
            }
            Tipo::Texto => {
                if s.escapado && self.ctx.estrito {
                    return self.erro_aqui("escape octal no modo estrito");
                }
                self.avancar()?;
                Ok(self.no(No::Texto(Self::texto_ref(&s)), pos))
            }
            Tipo::Template(_) => self.template(false),
            Tipo::Regex => {
                let Dado::Regex { corpo, flags } = s.dado else { unreachable!() };
                self.avancar()?;
                Ok(self.no(No::Regex { corpo, flags }, pos))
            }
            Tipo::Pont(Pont::Barra | Pont::BarraAtrib) => {
                self.atual = self.lex.reler_como_regex(&s)?;
                self.primaria()
            }
            Tipo::Pont(Pont::AbreParen) => {
                self.avancar()?;
                let sem_in = self.ctx.sem_in;
                self.ctx.sem_in = false;
                let e = self.expressao()?;
                self.ctx.sem_in = sem_in;
                self.esperar(Pont::FechaParen)?;
                Ok(self.no(No::Paren(e), pos))
            }
            Tipo::Pont(Pont::AbreColchete) => self.array(),
            Tipo::Pont(Pont::AbreChave) => self.objeto(),
            Tipo::Fim => self.erro_aqui("o código acabou no meio de uma expressão"),
            _ => self.erro_aqui("esperava uma expressão"),
        }
    }

    fn super_(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        match self.atual.tipo {
            Tipo::Pont(Pont::AbreParen) if self.ctx.super_chamada => {}
            Tipo::Pont(Pont::Ponto | Pont::AbreColchete) if self.ctx.super_prop => {}
            _ => return self.erro(pos, "'super' fora de método"),
        }
        self.marcar_funcao(|f| f.usa_this = true);
        Ok(self.no(No::Super, pos))
    }

    fn import_expr(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        if self.comer(Pont::Ponto)? {
            if !self.palavra(p::META) || !self.arv.modulo {
                return self.erro_aqui("import.meta só em módulo");
            }
            self.avancar()?;
            return Ok(self.no(No::ImportMeta, pos));
        }
        self.esperar(Pont::AbreParen)?;
        let arg = self.atribuicao_dentro()?;
        let mut opcoes = NoId::NADA;
        if self.comer(Pont::Virgula)? && !self.e(Pont::FechaParen) {
            opcoes = self.atribuicao_dentro()?;
            self.comer(Pont::Virgula)?;
        }
        self.esperar(Pont::FechaParen)?;
        Ok(self.no(No::ImportDinamico { arg, opcoes }, pos))
    }

    fn atribuicao_dentro(&mut self) -> R<NoId> {
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let e = self.atribuicao_fechada();
        self.ctx.sem_in = sem_in;
        e
    }

    /// Template (o símbolo atual é a cabeça). `marcado`: escapes inválidos permitidos.
    fn template(&mut self, marcado: bool) -> R<NoId> {
        let pos = self.atual.inicio;
        let base_p = self.rascunho.len();
        let mut exprs: Vec<NoId> = Vec::new();
        loop {
            let s = self.atual;
            let Tipo::Template(t) = s.tipo else { return self.erro_aqui("esperava continuação do template") };
            let Dado::Template(i) = s.dado else { unreachable!() };
            if !marcado && self.lex.template(i).cozido.is_none() {
                return self.erro(s.inicio, "escape inválido em template");
            }
            let pedaco = self.no(No::PedacoTemplate(i), s.inicio);
            self.rascunho.push(pedaco);
            if t.cauda {
                self.avancar()?;
                break;
            }
            self.avancar()?;
            let e = self.expressao_dentro()?;
            exprs.push(e);
            if !self.e(Pont::FechaChave) {
                return self.erro_aqui("esperava } fechando ${ no template");
            }
            let chave = self.atual;
            self.atual = self.lex.continuar_template(&chave)?;
        }
        let pedacos = self.lista(base_p);
        let base_e = self.rascunho.len();
        self.rascunho.extend(exprs);
        let exprs = self.lista(base_e);
        Ok(self.no(No::Template { pedacos, exprs }, pos))
    }

    fn array(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let base = self.rascunho.len();
        loop {
            if self.e(Pont::FechaColchete) {
                break;
            }
            let ip = self.atual.inicio;
            if self.e(Pont::Virgula) {
                self.avancar()?;
                let b = self.no(No::Buraco, ip);
                self.rascunho.push(b);
                continue;
            }
            let item = if self.comer(Pont::Reticencias)? {
                let x = self.atribuicao()?;
                self.no(No::Espalhar(x), ip)
            } else {
                self.atribuicao()?
            };
            self.rascunho.push(item);
            if !self.comer(Pont::Virgula)? {
                break;
            }
        }
        self.ctx.sem_in = sem_in;
        self.esperar(Pont::FechaColchete)?;
        let l = self.lista(base);
        Ok(self.no(No::Array(l), pos))
    }

    /// Chave de propriedade (objeto, classe, padrão).
    pub(crate) fn chave(&mut self) -> R<Chave> {
        let s = self.atual;
        let c = match (s.tipo, s.dado) {
            (Tipo::Nome, Dado::Atomo(a)) => Chave::Atomo(a),
            (Tipo::Texto, _) => {
                if s.escapado && self.ctx.estrito {
                    return self.erro_aqui("escape octal no modo estrito");
                }
                Chave::Texto(Self::texto_ref(&s))
            }
            (Tipo::Numero, Dado::Numero(n)) => Chave::Numero(n),
            (Tipo::BigInt, Dado::Texto(i)) => Chave::BigInt(i),
            (Tipo::NomePrivado, Dado::Atomo(a)) => Chave::Privada(a),
            (Tipo::Pont(Pont::AbreColchete), _) => {
                self.avancar()?;
                let e = self.atribuicao_dentro()?;
                self.esperar(Pont::FechaColchete)?;
                return Ok(Chave::Calculada(e));
            }
            _ => return self.erro_aqui("esperava nome de propriedade"),
        };
        self.avancar()?;
        Ok(c)
    }

    fn objeto(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let base = self.rascunho.len();
        while !self.e(Pont::FechaChave) {
            let item = self.propriedade()?;
            self.rascunho.push(item);
            if !self.comer(Pont::Virgula)? {
                break;
            }
        }
        self.ctx.sem_in = sem_in;
        self.esperar(Pont::FechaChave)?;
        let l = self.lista(base);
        Ok(self.no(No::Objeto(l), pos))
    }

    fn propriedade(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        if self.comer(Pont::Reticencias)? {
            let x = self.atribuicao()?;
            return Ok(self.no(No::Espalhar(x), pos));
        }
        // Prefixos: async, get, set, * (só se o que vem depois é chave, não `:`/`(`/`,`/`}`).
        let mut assincrona = false;
        let mut gerador = false;
        let mut tipo = TipoProp::Normal;
        if self.atual.tipo == Tipo::Nome && !self.atual.escapado {
            let a = self.atomo();
            if a == Some(p::ASYNC) || a == Some(p::GET) || a == Some(p::SET) {
                let prox = self.espiar()?;
                let vem_chave = !matches!(prox.tipo, Tipo::Pont(Pont::DoisPontos | Pont::AbreParen | Pont::Virgula | Pont::FechaChave | Pont::Atrib));
                if vem_chave && !(a == Some(p::ASYNC) && prox.quebra_antes) {
                    self.avancar()?;
                    match a {
                        Some(p::ASYNC) => assincrona = true,
                        Some(p::GET) => tipo = TipoProp::Get,
                        _ => tipo = TipoProp::Set,
                    }
                }
            }
        }
        if self.e(Pont::Vezes) && tipo == TipoProp::Normal {
            self.avancar()?;
            gerador = true;
        }
        let nome_simples = self.atual.tipo == Tipo::Nome;
        let s_chave = self.atual;
        let chave = self.chave()?;
        if let Chave::Privada(_) = chave {
            return self.erro(pos, "#nome em objeto literal");
        }
        // Método/getter/setter.
        if self.e(Pont::AbreParen) || tipo != TipoProp::Normal || assincrona || gerador {
            let tipo = if tipo == TipoProp::Normal { TipoProp::Metodo } else { tipo };
            let f = self.metodo(pos, assincrona, gerador, tipo == TipoProp::Get, tipo == TipoProp::Set, false, false)?;
            let valor = self.no(No::Funcao(f), pos);
            return Ok(self.no(No::Propriedade { chave, valor, tipo }, pos));
        }
        // shortcut: `__proto__` duplicado não é conferido (erro só fora de padrão; raro).
        if self.comer(Pont::DoisPontos)? {
            let valor = self.atribuicao()?;
            return Ok(self.no(No::Propriedade { chave, valor, tipo: TipoProp::Normal }, pos));
        }
        // `{ x }` e `{ x = 1 }` (este só em padrão).
        if !nome_simples {
            return self.erro_aqui("esperava : depois da chave");
        }
        let Chave::Atomo(a) = chave else { unreachable!() };
        if s_chave.escapado && reservada(a) {
            return self.erro(pos, "palavra reservada escapada");
        }
        self.checar_ident(a, pos)?;
        if a == p::ARGUMENTS {
            self.marcar_funcao(|f| f.usa_arguments = true);
        }
        let nome = self.no(No::Nome(a), pos);
        let valor = if self.e(Pont::Atrib) {
            self.pendentes.push(self.atual.inicio);
            self.avancar()?;
            let v = self.atribuicao()?;
            self.no(No::PadraoComValor { alvo: nome, valor: v }, pos)
        } else {
            nome
        };
        Ok(self.no(No::Propriedade { chave, valor, tipo: TipoProp::Curta }, pos))
    }

    /// Tenta ler uma seta no símbolo atual; devolve `None` (sem consumir nada) se não é.
    fn tentar_seta(&mut self) -> R<Option<NoId>> {
        let pos = self.atual.inicio;
        let mut assincrona = false;
        match self.atual.tipo {
            Tipo::Nome => {
                // `x =>` é visto depois de ler `x` (em `atribuicao`), sem espiar a cada nome.
                if !self.palavra(p::ASYNC) {
                    return Ok(None);
                }
                let prox = self.espiar()?;
                if prox.quebra_antes || prox.tipo == Tipo::Pont(Pont::Seta) {
                    return Ok(None);
                }
                // `async x =>` / `async (…) =>`
                if prox.tipo == Tipo::Nome {
                    let m = self.marca();
                    self.avancar()?;
                    let s = self.atual;
                    let Dado::Atomo(a) = s.dado else { unreachable!() };
                    let prox2 = self.espiar()?;
                    if prox2.tipo == Tipo::Pont(Pont::Seta) && !prox2.quebra_antes {
                        if a == p::AWAIT {
                            return self.erro(s.inicio, "'await' como parâmetro de seta async");
                        }
                        self.checar_vinculo(a, s.inicio)?;
                        self.avancar()?;
                        let param = self.no(No::Nome(a), s.inicio);
                        return self.seta(pos, true, ParamsSeta::Um(param)).map(Some);
                    }
                    self.voltar(m);
                    return Ok(None);
                }
                if prox.tipo != Tipo::Pont(Pont::AbreParen) {
                    return Ok(None);
                }
                assincrona = true;
            }
            Tipo::Pont(Pont::AbreParen) => {}
            _ => return Ok(None),
        }
        // `(…) =>`: tenta ler parâmetros e ver se vem `=>`.
        if self.nao_e_seta.contains_key(&pos) {
            return Ok(None);
        }
        let m = self.marca();
        if assincrona {
            self.avancar()?;
        }
        let ctx = self.ctx;
        self.ctx.assincrona = assincrona;
        self.ctx.em_campo = false;
        let params = self.parametros();
        self.ctx = ctx;
        match params {
            Ok((lista, simples)) if self.e(Pont::Seta) && !self.atual.quebra_antes => {
                if assincrona {
                    // `async (await) =>` é erro.
                    for &x in self.arv.lista(lista).to_vec().iter() {
                        if self.contem_nome(x, p::AWAIT) {
                            return self.erro(self.arv.posicao(x), "'await' em parâmetro de seta async");
                        }
                    }
                }
                self.seta(pos, assincrona, ParamsSeta::Lista(lista, simples)).map(Some)
            }
            _ => {
                self.voltar(m);
                self.nao_e_seta.insert(pos, ());
                Ok(None)
            }
        }
    }

    fn contem_nome(&self, id: NoId, a: Atomo) -> bool {
        match *self.arv.no(id) {
            No::Nome(n) => n == a,
            No::PadraoComValor { alvo, valor } => self.contem_nome(alvo, a) || self.contem_nome(valor, a),
            No::PadraoResto(x) => self.contem_nome(x, a),
            No::PadraoArray(l) | No::PadraoObjeto(l) => self.arv.lista(l).iter().any(|&x| self.contem_nome(x, a)),
            No::Propriedade { valor, .. } => self.contem_nome(valor, a),
            No::Await(_) => true,
            _ => false,
        }
    }
}

pub(crate) enum ParamsSeta {
    Um(NoId),
    Lista(Lista, bool),
}

#[derive(Clone, Copy)]
enum Op {
    Bin(OpBinario),
    Log(OpLogico),
}

fn op_atribuicao(pt: Pont) -> Option<OpAtrib> {
    use OpBinario as B;
    Some(match pt {
        Pont::Atrib => OpAtrib::Simples,
        Pont::MaisAtrib => OpAtrib::Bin(B::Soma),
        Pont::MenosAtrib => OpAtrib::Bin(B::Sub),
        Pont::VezesAtrib => OpAtrib::Bin(B::Mul),
        Pont::BarraAtrib => OpAtrib::Bin(B::Div),
        Pont::RestoAtrib => OpAtrib::Bin(B::Resto),
        Pont::PotenciaAtrib => OpAtrib::Bin(B::Potencia),
        Pont::DeslEsqAtrib => OpAtrib::Bin(B::DeslEsq),
        Pont::DeslDirAtrib => OpAtrib::Bin(B::DeslDir),
        Pont::DeslDirSemSinalAtrib => OpAtrib::Bin(B::DeslDirSemSinal),
        Pont::EAtrib => OpAtrib::Bin(B::E),
        Pont::OuAtrib => OpAtrib::Bin(B::Ou),
        Pont::OuExclusivoAtrib => OpAtrib::Bin(B::OuExclusivo),
        Pont::EEAtrib => OpAtrib::Log(OpLogico::E),
        Pont::OuOuAtrib => OpAtrib::Log(OpLogico::Ou),
        Pont::CoalesceAtrib => OpAtrib::Log(OpLogico::Coalesce),
        _ => return None,
    })
}


pub(crate) fn texto_pont(pt: Pont) -> &'static str {
    use Pont::*;
    match pt {
        AbreChave => "{", FechaChave => "}", AbreParen => "(", FechaParen => ")",
        AbreColchete => "[", FechaColchete => "]", PontoVirgula => ";", Virgula => ",",
        DoisPontos => ":", Seta => "=>", Atrib => "=", Ponto => ".",
        _ => "pontuação",
    }
}
