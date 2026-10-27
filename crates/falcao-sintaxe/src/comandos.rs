//! Comandos, funções, classes e módulos (ECMA-262 seções 14 a 16). As expressões estão em
//! `sintaxe.rs`.

use crate::arvore::*;
use crate::atomos::{palavras as p, Atomo};
use crate::lexico::{Dado, Pont, Tipo};
use crate::sintaxe::{Analisador, Ctx, ParamsSeta, R};

impl<'a> Analisador<'a> {
    // ---------------------------------------------------------------- programa

    pub(crate) fn programa(&mut self) -> R<Lista> {
        let base = self.rascunho.len();
        self.diretivas(base)?;
        self.arv.estrito = self.ctx.estrito;
        while self.atual.tipo != Tipo::Fim {
            let c = if self.arv.modulo { self.item_modulo()? } else { self.item()? };
            self.rascunho.push(c);
        }
        Ok(self.lista(base))
    }

    /// Prólogo de diretivas (`"use strict";` no começo do corpo). Os comandos lidos vão para o
    /// rascunho a partir de `base`; devolve se ligou o modo estrito.
    fn diretivas(&mut self, base: usize) -> R<bool> {
        let mut ligou = false;
        while self.atual.tipo == Tipo::Texto {
            let s = self.atual;
            let c = self.item()?;
            self.rascunho.push(c);
            let No::Expressao(e) = *self.arv.no(c) else { break };
            if !matches!(self.arv.no(e), No::Texto(_)) {
                break;
            }
            if let Dado::Faixa(a, b) = s.dado {
                if &self.lex.fonte()[a as usize..b as usize] == "use strict" {
                    self.ctx.estrito = true;
                    ligou = true;
                }
            }
        }
        let _ = base;
        Ok(ligou)
    }

    // ---------------------------------------------------------------- comandos

    /// `StatementListItem`: comando ou declaração.
    pub(crate) fn item(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        if self.atual.tipo == Tipo::Nome && !self.atual.escapado {
            match self.atomo() {
                Some(p::FUNCTION) => return self.decl_funcao(false, pos, false),
                Some(p::CLASS) => {
                    let c = self.classe(true)?;
                    return Ok(self.no(No::DeclClasse(c), pos));
                }
                Some(p::CONST) => return self.decl_var(TipoVar::Const, true),
                Some(p::LET) if self.let_declara()? => return self.decl_var(TipoVar::Let, true),
                Some(p::ASYNC) => {
                    let prox = self.espiar()?;
                    if prox.tipo == Tipo::Nome && prox.dado == Dado::Atomo(p::FUNCTION) && !prox.quebra_antes && !prox.escapado {
                        self.avancar()?;
                        return self.decl_funcao(true, pos, false);
                    }
                }
                _ => {}
            }
        }
        self.comando()
    }

    /// `let` seguido de nome, `[` ou `{` declara; senão é nome (modo não estrito).
    fn let_declara(&mut self) -> R<bool> {
        if self.ctx.estrito {
            return Ok(true);
        }
        let prox = self.espiar()?;
        Ok(match prox.tipo {
            Tipo::Pont(Pont::AbreColchete | Pont::AbreChave) => true,
            Tipo::Nome => !prox.quebra_antes || !matches!(prox.dado, Dado::Atomo(a) if a == p::IN || a == p::INSTANCEOF || a == p::OF),
            _ => false,
        })
    }

    pub(crate) fn comando(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        match self.atual.tipo {
            Tipo::Pont(Pont::AbreChave) => return self.bloco(),
            Tipo::Pont(Pont::PontoVirgula) => {
                self.avancar()?;
                return Ok(self.no(No::Vazio, pos));
            }
            Tipo::Nome if !self.atual.escapado => {}
            _ => return self.comando_expressao(),
        }
        let a = self.atomo().unwrap();
        match a {
            p::VAR => self.decl_var(TipoVar::Var, true),
            p::IF => {
                self.avancar()?;
                let teste = self.cond_entre_parenteses()?;
                let sim = self.corpo_sub()?;
                let nao = if self.comer_palavra(p::ELSE)? { self.corpo_sub()? } else { NoId::NADA };
                Ok(self.no(No::Se { teste, sim, nao }, pos))
            }
            p::FOR => self.para(),
            p::WHILE => {
                self.avancar()?;
                let teste = self.cond_entre_parenteses()?;
                let corpo = self.corpo_laco()?;
                Ok(self.no(No::Enquanto { teste, corpo }, pos))
            }
            p::DO => {
                self.avancar()?;
                let corpo = self.corpo_laco()?;
                self.esperar_palavra(p::WHILE)?;
                let teste = self.cond_entre_parenteses()?;
                // Depois de `do … while (x)` o `;` é sempre opcional (§12.10.1).
                self.comer(Pont::PontoVirgula)?;
                Ok(self.no(No::FacaEnquanto { corpo, teste }, pos))
            }
            p::CONTINUE | p::BREAK => {
                self.avancar()?;
                let mut rotulo = None;
                if !self.atual.quebra_antes {
                    if let Some(r) = self.atomo() {
                        if !crate::atomos::reservada(r) || self.atual.escapado {
                            rotulo = Some(r);
                            self.avancar()?;
                        }
                    }
                }
                let continua = a == p::CONTINUE;
                match rotulo {
                    Some(r) => match self.rotulos.iter().rev().find(|(n, _)| *n == r) {
                        None => return self.erro(pos, "rótulo não encontrado"),
                        Some(&(_, laco)) if continua && !laco => return self.erro(pos, "continue com rótulo que não é de laço"),
                        _ => {}
                    },
                    None if continua && !self.ctx.em_laco => return self.erro(pos, "continue fora de laço"),
                    None if !continua && !self.ctx.em_laco && !self.ctx.em_escolha => return self.erro(pos, "break fora de laço ou switch"),
                    None => {}
                }
                self.fim_comando()?;
                Ok(self.no(if continua { No::Continue(rotulo) } else { No::Break(rotulo) }, pos))
            }
            p::RETURN => {
                if !self.ctx.em_funcao {
                    return self.erro_aqui("return fora de função");
                }
                self.avancar()?;
                let arg = if self.e(Pont::PontoVirgula) || self.e(Pont::FechaChave) || self.atual.tipo == Tipo::Fim || self.atual.quebra_antes {
                    NoId::NADA
                } else {
                    self.expressao_fechada()?
                };
                self.fim_comando()?;
                Ok(self.no(No::Retorne(arg), pos))
            }
            p::WITH => {
                if self.ctx.estrito {
                    return self.erro_aqui("with no modo estrito");
                }
                self.avancar()?;
                let obj = self.cond_entre_parenteses()?;
                let corpo = self.corpo_sub()?;
                self.marcar_funcao(|f| f.eval_direto = true);
                Ok(self.no(No::Com { obj, corpo }, pos))
            }
            p::SWITCH => self.escolha(),
            p::THROW => {
                self.avancar()?;
                if self.atual.quebra_antes {
                    return self.erro_aqui("quebra de linha depois de throw");
                }
                let arg = self.expressao_fechada()?;
                self.fim_comando()?;
                Ok(self.no(No::Lance(arg), pos))
            }
            p::TRY => self.tente(),
            p::DEBUGGER => {
                self.avancar()?;
                self.fim_comando()?;
                Ok(self.no(No::Depurador, pos))
            }
            // Anexo B: `if (x) function f(){}` no modo não estrito.
            p::FUNCTION if !self.ctx.estrito => self.decl_funcao(false, pos, false),
            p::FUNCTION | p::CLASS => self.erro_aqui("declaração não pode ser corpo de comando"),
            p::CONST => self.erro_aqui("declaração não pode ser corpo de comando"),
            p::IMPORT => {
                let prox = self.espiar()?;
                if matches!(prox.tipo, Tipo::Pont(Pont::AbreParen | Pont::Ponto)) {
                    self.comando_expressao()
                } else {
                    self.erro_aqui("import só no topo de módulo")
                }
            }
            p::EXPORT => self.erro_aqui("export só no topo de módulo"),
            _ => self.comando_expressao(),
        }
    }

    /// Comando de expressão ou rótulo (`nome: comando`).
    fn comando_expressao(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        let e = self.expressao_fechada()?;
        if self.e(Pont::DoisPontos) {
            if let No::Nome(r) = *self.arv.no(e) {
                {
                    self.avancar()?;
                    if self.rotulos.iter().any(|(n, _)| *n == r) {
                        return self.erro(pos, "rótulo repetido");
                    }
                    let e_laco = self.palavra(p::FOR) || self.palavra(p::WHILE) || self.palavra(p::DO);
                    self.rotulos.push((r, e_laco));
                    let corpo = if self.palavra(p::FUNCTION) && !self.ctx.estrito {
                        let fp = self.atual.inicio;
                        self.decl_funcao(false, fp, false)
                    } else {
                        self.comando()
                    };
                    self.rotulos.pop();
                    let corpo = corpo?;
                    return Ok(self.no(No::Rotulo { rotulo: r, corpo }, pos));
                }
            }
        }
        self.fim_comando()?;
        Ok(self.no(No::Expressao(e), pos))
    }

    fn bloco(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.esperar(Pont::AbreChave)?;
        let base = self.rascunho.len();
        while !self.e(Pont::FechaChave) {
            if self.atual.tipo == Tipo::Fim {
                return self.erro_aqui("esperava }");
            }
            let c = self.item()?;
            self.rascunho.push(c);
        }
        self.avancar()?;
        let l = self.lista(base);
        Ok(self.no(No::Bloco(l), pos))
    }

    fn cond_entre_parenteses(&mut self) -> R<NoId> {
        self.esperar(Pont::AbreParen)?;
        let e = self.expressao_dentro()?;
        self.esperar(Pont::FechaParen)?;
        Ok(e)
    }

    /// Corpo de `if`/`with`/rótulo: comando simples.
    fn corpo_sub(&mut self) -> R<NoId> {
        self.comando()
    }

    fn corpo_laco(&mut self) -> R<NoId> {
        let antes = self.ctx.em_laco;
        self.ctx.em_laco = true;
        let c = self.comando();
        self.ctx.em_laco = antes;
        c
    }

    /// `var/let/const` com declaradores; `fim`: consome o `;` (falso dentro de `for (`).
    fn decl_var(&mut self, tipo: TipoVar, fim: bool) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let base = self.rascunho.len();
        loop {
            let dp = self.atual.inicio;
            let alvo = self.alvo_vinculo()?;
            if tipo != TipoVar::Var {
                if let No::Nome(a) = *self.arv.no(alvo) {
                    if a == p::LET {
                        return self.erro(dp, "'let' não pode ser nome de let/const");
                    }
                }
            }
            let valor = if self.comer(Pont::Atrib)? {
                self.atribuicao_fechada()?
            } else {
                let em_for = !fim && (self.palavra(p::IN) || self.palavra(p::OF));
                if !em_for && (tipo == TipoVar::Const || !matches!(self.arv.no(alvo), No::Nome(_))) {
                    return self.erro_aqui("declaração precisa de valor inicial");
                }
                NoId::NADA
            };
            let d = self.no(No::Declarador { alvo, valor }, dp);
            self.rascunho.push(d);
            if !self.comer(Pont::Virgula)? {
                break;
            }
        }
        if fim {
            self.fim_comando()?;
        }
        let decls = self.lista(base);
        Ok(self.no(No::Var { tipo, decls }, pos))
    }

    /// Alvo de declaração/parâmetro: nome, `[...]` ou `{...}`.
    pub(crate) fn alvo_vinculo(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        match self.atual.tipo {
            Tipo::Pont(Pont::AbreColchete | Pont::AbreChave) => {
                let antes = self.pendentes.len();
                let sem_in = self.ctx.sem_in;
                let e = self.lhs()?;
                self.ctx.sem_in = sem_in;
                self.para_padrao(e, true)?;
                self.pendentes.truncate(antes);
                Ok(e)
            }
            _ => {
                let Some(a) = self.atomo() else { return self.erro_aqui("esperava um nome") };
                self.checar_vinculo(a, pos)?;
                self.avancar()?;
                Ok(self.no(No::Nome(a), pos))
            }
        }
    }

    fn para(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let aguarda = if self.palavra(p::AWAIT) {
            if !(self.ctx.assincrona || (self.ctx.modulo && !self.ctx.em_funcao)) {
                return self.erro_aqui("for await fora de função async");
            }
            self.arv.await_topo |= !self.ctx.em_funcao;
            self.avancar()?;
            true
        } else {
            false
        };
        self.esperar(Pont::AbreParen)?;
        // Início: vazio, declaração ou expressão (com `in` desligado).
        let mut inicio = NoId::NADA;
        let mut decl = false;
        if !self.e(Pont::PontoVirgula) {
            let sem_in = self.ctx.sem_in;
            self.ctx.sem_in = true;
            let r = (|| -> R<NoId> {
                if self.palavra(p::VAR) {
                    decl = true;
                    return self.decl_var(TipoVar::Var, false);
                }
                if self.palavra(p::CONST) {
                    decl = true;
                    return self.decl_var(TipoVar::Const, false);
                }
                if self.palavra(p::LET) && self.let_declara_for()? {
                    decl = true;
                    return self.decl_var(TipoVar::Let, false);
                }
                let antes = self.pendentes.len();
                let e = self.expressao()?;
                if self.palavra(p::OF) || self.palavra(p::IN) {
                    if matches!(self.arv.no(e), No::Array(_) | No::Objeto(_)) {
                        self.para_padrao(e, false)?;
                        self.pendentes.truncate(antes);
                    } else {
                        self.conferir_pendentes(antes)?;
                        self.alvo_simples(e)?;
                    }
                } else {
                    self.conferir_pendentes(antes)?;
                }
                Ok(e)
            })();
            self.ctx.sem_in = sem_in;
            inicio = r?;
        }
        if self.palavra(p::OF) || self.palavra(p::IN) {
            let de = self.palavra(p::OF);
            if decl {
                if let No::Var { decls, .. } = *self.arv.no(inicio) {
                    if decls.len != 1 {
                        return self.erro_aqui("for-in/of com mais de um declarador");
                    }
                }
            }
            self.avancar()?;
            let dir = if de { self.atribuicao_dentro_pub()? } else { self.expressao_dentro()? };
            self.esperar(Pont::FechaParen)?;
            let corpo = self.corpo_laco()?;
            return Ok(if de {
                self.no(No::ParaDe { esq: inicio, dir, corpo, aguarda }, pos)
            } else {
                self.no(No::ParaEm { esq: inicio, dir, corpo }, pos)
            });
        }
        if aguarda {
            return self.erro_aqui("for await precisa de of");
        }
        // Declaração sem valor só vale em for-in/of: confere aqui.
        if decl {
            if let No::Var { tipo, decls } = *self.arv.no(inicio) {
                for &d in self.arv.lista(decls) {
                    if let No::Declarador { alvo, valor } = *self.arv.no(d) {
                        if !valor.existe() && (tipo == TipoVar::Const || !matches!(self.arv.no(alvo), No::Nome(_))) {
                            return self.erro(self.arv.posicao(d), "declaração precisa de valor inicial");
                        }
                    }
                }
            }
        }
        self.esperar(Pont::PontoVirgula)?;
        let teste = if self.e(Pont::PontoVirgula) { NoId::NADA } else { self.expressao_dentro()? };
        self.esperar(Pont::PontoVirgula)?;
        let passo = if self.e(Pont::FechaParen) { NoId::NADA } else { self.expressao_dentro()? };
        self.esperar(Pont::FechaParen)?;
        let corpo = self.corpo_laco()?;
        Ok(self.no(No::Para { inicio, teste, passo, corpo }, pos))
    }

    /// Em `for (let`, `let` declara se vem nome, `[` ou `{` (não `for (let in x)`).
    fn let_declara_for(&mut self) -> R<bool> {
        if self.ctx.estrito {
            return Ok(true);
        }
        let prox = self.espiar()?;
        Ok(match prox.tipo {
            Tipo::Pont(Pont::AbreColchete | Pont::AbreChave) => true,
            Tipo::Nome => !matches!(prox.dado, Dado::Atomo(a) if a == p::IN || a == p::OF),
            _ => false,
        })
    }

    pub(crate) fn atribuicao_dentro_pub(&mut self) -> R<NoId> {
        let sem_in = self.ctx.sem_in;
        self.ctx.sem_in = false;
        let e = self.atribuicao_fechada();
        self.ctx.sem_in = sem_in;
        e
    }

    fn escolha(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let disc = self.cond_entre_parenteses()?;
        self.esperar(Pont::AbreChave)?;
        let antes = self.ctx.em_escolha;
        self.ctx.em_escolha = true;
        let base_c = self.rascunho.len();
        let mut tem_padrao = false;
        while !self.e(Pont::FechaChave) {
            let cp = self.atual.inicio;
            let teste = if self.comer_palavra(p::CASE)? {
                self.expressao_dentro()?
            } else if self.comer_palavra(p::DEFAULT)? {
                if tem_padrao {
                    return self.erro(cp, "dois default no switch");
                }
                tem_padrao = true;
                NoId::NADA
            } else {
                return self.erro_aqui("esperava case ou default");
            };
            self.esperar(Pont::DoisPontos)?;
            let base = self.rascunho.len();
            while !self.e(Pont::FechaChave) && !self.palavra(p::CASE) && !self.palavra(p::DEFAULT) {
                if self.atual.tipo == Tipo::Fim {
                    return self.erro_aqui("esperava }");
                }
                let c = self.item()?;
                self.rascunho.push(c);
            }
            let corpo = self.lista(base);
            let caso = self.no(No::Caso { teste, corpo }, cp);
            self.rascunho.push(caso);
        }
        self.avancar()?;
        self.ctx.em_escolha = antes;
        let casos = self.lista(base_c);
        Ok(self.no(No::Escolha { disc, casos }, pos))
    }

    fn tente(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let bloco = self.bloco()?;
        let mut param = NoId::NADA;
        let mut captura = NoId::NADA;
        let mut finalmente = NoId::NADA;
        if self.comer_palavra(p::CATCH)? {
            if self.comer(Pont::AbreParen)? {
                param = self.alvo_vinculo()?;
                self.esperar(Pont::FechaParen)?;
            }
            captura = self.bloco()?;
        }
        if self.comer_palavra(p::FINALLY)? {
            finalmente = self.bloco()?;
        }
        if !captura.existe() && !finalmente.existe() {
            return self.erro_aqui("try sem catch nem finally");
        }
        Ok(self.no(No::Tente { bloco, param, captura, finalmente }, pos))
    }

    // ---------------------------------------------------------------- funções

    /// Abre uma função na arena e entra nela (contexto, rótulos). Devolve o que restaurar.
    fn entrar_funcao(&mut self, nome: Option<Atomo>, flags: FlagsFuncao, inicio: u32) -> (FuncaoId, Option<FuncaoId>, Ctx, Vec<(Atomo, bool)>) {
        let id = FuncaoId(self.arv.funcoes.len() as u32);
        self.arv.funcoes.push(Funcao { nome, params: Lista::default(), corpo: Lista::default(), flags, inicio, fim: inicio, pai: self.funcao });
        let pai = self.funcao.replace(id);
        let ctx = self.ctx;
        let rotulos = std::mem::take(&mut self.rotulos);
        self.ctx.em_laco = false;
        self.ctx.em_escolha = false;
        self.ctx.sem_in = false;
        self.ctx.em_campo = false;
        self.ctx.em_funcao = true;
        if !flags.seta {
            self.ctx.assincrona = flags.assincrona;
            self.ctx.gerador = flags.gerador;
            self.ctx.new_target = true;
            self.ctx.super_prop = flags.metodo;
            self.ctx.super_chamada = flags.construtor_classe && flags.classe_derivada;
        } else {
            self.ctx.assincrona = flags.assincrona;
            self.ctx.gerador = false;
        }
        (id, pai, ctx, rotulos)
    }

    fn sair_funcao(&mut self, id: FuncaoId, pai: Option<FuncaoId>, ctx: Ctx, rotulos: Vec<(Atomo, bool)>) {
        let f = &mut self.arv.funcoes[id.0 as usize];
        f.fim = self.fim_anterior;
        f.flags.estrita = self.ctx.estrito;
        self.funcao = pai;
        self.ctx = ctx;
        self.rotulos = rotulos;
    }

    /// `( parâmetros )`. Devolve a lista e se são todos nomes simples.
    pub(crate) fn parametros(&mut self) -> R<(Lista, bool)> {
        self.esperar(Pont::AbreParen)?;
        let base = self.rascunho.len();
        let mut simples = true;
        while !self.e(Pont::FechaParen) {
            let pos = self.atual.inicio;
            if self.comer(Pont::Reticencias)? {
                let x = self.alvo_vinculo()?;
                if self.e(Pont::Atrib) {
                    return self.erro_aqui("resto (...) não aceita valor padrão");
                }
                let r = self.no(No::PadraoResto(x), pos);
                self.rascunho.push(r);
                simples = false;
                if !self.e(Pont::FechaParen) {
                    return self.erro_aqui("o resto (...) tem de ser o último parâmetro");
                }
                break;
            }
            let alvo = self.alvo_vinculo()?;
            let item = if self.comer(Pont::Atrib)? {
                simples = false;
                let valor = self.atribuicao_dentro_pub()?;
                self.no(No::PadraoComValor { alvo, valor }, pos)
            } else {
                if !matches!(self.arv.no(alvo), No::Nome(_)) {
                    simples = false;
                }
                alvo
            };
            self.rascunho.push(item);
            if !self.comer(Pont::Virgula)? {
                break;
            }
        }
        self.esperar(Pont::FechaParen)?;
        Ok((self.lista(base), simples))
    }

    /// `{ corpo }` de função, com diretivas.
    fn corpo_funcao(&mut self, id: FuncaoId) -> R<()> {
        self.esperar(Pont::AbreChave)?;
        let base = self.rascunho.len();
        let estava = self.ctx.estrito;
        if self.diretivas(base)? && !estava && !self.arv.funcoes[id.0 as usize].flags.parametros_simples {
            return self.erro_aqui("\"use strict\" em função com parâmetros não simples");
        }
        while !self.e(Pont::FechaChave) {
            if self.atual.tipo == Tipo::Fim {
                return self.erro_aqui("esperava } fechando a função");
            }
            let c = self.item()?;
            self.rascunho.push(c);
        }
        self.avancar()?;
        let corpo = self.lista(base);
        self.arv.funcoes[id.0 as usize].corpo = corpo;
        Ok(())
    }

    /// Parâmetros + corpo de uma função já aberta.
    fn resto_funcao(&mut self, id: FuncaoId) -> R<()> {
        let (params, simples) = self.parametros()?;
        let f = &mut self.arv.funcoes[id.0 as usize];
        f.params = params;
        f.flags.parametros_simples = simples;
        self.corpo_funcao(id)
    }

    /// `function` [`*`] nome? `(…) {…}`; `async` já consumido (`assincrona`).
    fn funcao(&mut self, assincrona: bool, inicio: u32, declaracao: bool, nome_opcional: bool) -> R<FuncaoId> {
        self.esperar_palavra(p::FUNCTION)?;
        let gerador = self.comer(Pont::Vezes)?;
        let mut nome = None;
        if self.atual.tipo == Tipo::Nome && !self.e(Pont::AbreParen) {
            let a = self.atomo().unwrap();
            let np = self.atual.inicio;
            if declaracao {
                self.checar_vinculo(a, np)?;
            } else {
                // Nome de expressão de função: vale o contexto da própria função.
                let ctx = self.ctx;
                self.ctx.gerador = gerador;
                self.ctx.assincrona = assincrona;
                let r = self.checar_vinculo(a, np);
                self.ctx = ctx;
                r?;
            }
            self.avancar()?;
            nome = Some(a);
        } else if declaracao && !nome_opcional {
            return self.erro_aqui("função sem nome");
        }
        let flags = FlagsFuncao { assincrona, gerador, expressao_nomeada: !declaracao && nome.is_some(), ..FlagsFuncao::default() };
        let (id, pai, ctx, rotulos) = self.entrar_funcao(nome, flags, inicio);
        let r = self.resto_funcao(id);
        self.sair_funcao(id, pai, ctx, rotulos);
        r?;
        Ok(id)
    }

    pub(crate) fn expressao_funcao(&mut self, assincrona: bool, pos: u32) -> R<NoId> {
        let f = self.funcao(assincrona, pos, false, false)?;
        Ok(self.no(No::Funcao(f), pos))
    }

    fn decl_funcao(&mut self, assincrona: bool, pos: u32, nome_opcional: bool) -> R<NoId> {
        let f = self.funcao(assincrona, pos, true, nome_opcional)?;
        Ok(self.no(No::DeclFuncao(f), pos))
    }

    /// Método de objeto/classe: o símbolo atual é `(`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn metodo(&mut self, inicio: u32, assincrona: bool, gerador: bool, getter: bool, setter: bool, construtor: bool, derivada: bool) -> R<FuncaoId> {
        let flags = FlagsFuncao {
            assincrona,
            gerador,
            metodo: true,
            getter,
            setter,
            construtor_classe: construtor,
            classe_derivada: derivada,
            ..FlagsFuncao::default()
        };
        let (id, pai, ctx, rotulos) = self.entrar_funcao(None, flags, inicio);
        let r = self.resto_funcao(id);
        self.sair_funcao(id, pai, ctx, rotulos);
        r?;
        let f = &self.arv.funcoes[id.0 as usize];
        if getter && f.params.len != 0 {
            return self.erro(inicio, "getter não tem parâmetros");
        }
        if setter && f.params.len != 1 {
            return self.erro(inicio, "setter tem um parâmetro só");
        }
        Ok(id)
    }

    /// Corpo de seta, depois dos parâmetros (o símbolo atual é `=>`).
    pub(crate) fn seta(&mut self, inicio: u32, assincrona: bool, params: ParamsSeta) -> R<NoId> {
        self.esperar(Pont::Seta)?;
        let (lista, simples) = match params {
            ParamsSeta::Um(x) => {
                let base = self.rascunho.len();
                self.rascunho.push(x);
                (self.lista(base), true)
            }
            ParamsSeta::Lista(l, s) => (l, s),
        };
        let flags = FlagsFuncao { assincrona, seta: true, parametros_simples: simples, ..FlagsFuncao::default() };
        let sem_in = self.ctx.sem_in;
        let (id, pai, ctx, rotulos) = self.entrar_funcao(None, flags, inicio);
        self.arv.funcoes[id.0 as usize].params = lista;
        let r = if self.e(Pont::AbreChave) {
            self.corpo_funcao(id)
        } else {
            self.ctx.sem_in = sem_in;
            let ep = self.atual.inicio;
            match self.atribuicao_fechada() {
                Ok(e) => {
                    let ret = self.no(No::Retorne(e), ep);
                    let base = self.rascunho.len();
                    self.rascunho.push(ret);
                    let corpo = self.lista(base);
                    self.arv.funcoes[id.0 as usize].corpo = corpo;
                    Ok(())
                }
                Err(e) => Err(e),
            }
        };
        self.sair_funcao(id, pai, ctx, rotulos);
        r?;
        Ok(self.no(No::Funcao(id), inicio))
    }

    // ---------------------------------------------------------------- classes

    /// `class` nome? [`extends` expr] `{ membros }`.
    pub(crate) fn classe(&mut self, declaracao: bool) -> R<ClasseId> {
        let inicio = self.atual.inicio;
        self.esperar_palavra(p::CLASS)?;
        let ctx_fora = self.ctx;
        self.ctx.estrito = true;
        let r = self.classe_dentro(inicio, declaracao);
        self.ctx = ctx_fora;
        r
    }

    fn classe_dentro(&mut self, inicio: u32, declaracao: bool) -> R<ClasseId> {
        let mut nome = None;
        if self.atual.tipo == Tipo::Nome && !self.palavra(p::EXTENDS) {
            let a = self.atomo().unwrap();
            self.checar_vinculo(a, self.atual.inicio)?;
            self.avancar()?;
            nome = Some(a);
        } else if declaracao {
            return self.erro_aqui("classe sem nome");
        }
        let heranca = if self.comer_palavra(p::EXTENDS)? { self.lhs()? } else { NoId::NADA };
        let derivada = heranca.existe();
        self.esperar(Pont::AbreChave)?;
        let em_classe = self.ctx.em_classe;
        self.ctx.em_classe = true;
        let mut membros = Vec::new();
        let mut construtor = None;
        while !self.e(Pont::FechaChave) {
            if self.comer(Pont::PontoVirgula)? {
                continue;
            }
            if self.atual.tipo == Tipo::Fim {
                return self.erro_aqui("esperava } fechando a classe");
            }
            let m = self.membro_classe(derivada)?;
            if m.tipo == TipoMembroClasse::Metodo && !m.estatico && self.e_construtor(&m.chave) {
                if construtor.is_some() {
                    return self.erro(self.arv.posicao(m.valor), "dois constructor na classe");
                }
                let No::Funcao(f) = *self.arv.no(m.valor) else { unreachable!() };
                construtor = Some(f);
                continue;
            }
            membros.push(m);
        }
        self.avancar()?;
        self.ctx.em_classe = em_classe;
        let id = ClasseId(self.arv.classes.len() as u32);
        self.arv.classes.push(Classe { nome, heranca, construtor, membros, inicio, fim: self.fim_anterior });
        Ok(id)
    }

    fn e_construtor(&self, c: &Chave) -> bool {
        match c {
            Chave::Atomo(a) => *a == p::CONSTRUCTOR,
            Chave::Texto(TextoRef::Faixa(a, b)) => &self.lex.fonte()[*a as usize..*b as usize] == "constructor",
            _ => false,
        }
    }

    /// O símbolo seguinte é uma chave de membro (e não `(`/`=`/`;`/`}`)? Decide se `static`,
    /// `get`, `async`... são modificadores ou o próprio nome.
    fn vem_chave(&mut self) -> R<bool> {
        let prox = self.espiar()?;
        Ok(!matches!(prox.tipo, Tipo::Pont(Pont::AbreParen | Pont::Atrib | Pont::PontoVirgula | Pont::FechaChave)) && prox.tipo != Tipo::Fim)
    }

    fn membro_classe(&mut self, derivada: bool) -> R<MembroClasse> {
        let pos = self.atual.inicio;
        let mut estatico = false;
        if self.palavra(p::STATIC) {
            let prox = self.espiar()?;
            if prox.tipo == Tipo::Pont(Pont::AbreChave) {
                // `static { … }`: função sem parâmetros, rodada uma vez.
                self.avancar()?;
                let flags = FlagsFuncao { metodo: true, parametros_simples: true, ..FlagsFuncao::default() };
                let (id, pai, ctx, rotulos) = self.entrar_funcao(None, flags, pos);
                self.ctx.em_funcao = true;
                self.ctx.em_campo = true;
                let r = self.corpo_bloco_estatico(id);
                self.sair_funcao(id, pai, ctx, rotulos);
                r?;
                let valor = self.no(No::Funcao(id), pos);
                return Ok(MembroClasse { chave: Chave::Atomo(p::STATIC), tipo: TipoMembroClasse::BlocoEstatico, estatico: true, valor });
            }
            if self.vem_chave()? {
                self.avancar()?;
                estatico = true;
            }
        }
        let mut assincrona = false;
        let mut gerador = false;
        let mut tipo = TipoMembroClasse::Metodo;
        if self.atual.tipo == Tipo::Nome && !self.atual.escapado {
            let a = self.atomo();
            if matches!(a, Some(p::ASYNC | p::GET | p::SET | p::ACCESSOR)) {
                let prox = self.espiar()?;
                let vem = !matches!(prox.tipo, Tipo::Pont(Pont::AbreParen | Pont::Atrib | Pont::PontoVirgula | Pont::FechaChave)) && prox.tipo != Tipo::Fim;
                if vem && !prox.quebra_antes || vem && a != Some(p::ASYNC) && a != Some(p::ACCESSOR) {
                    self.avancar()?;
                    match a {
                        Some(p::ASYNC) => assincrona = true,
                        Some(p::GET) => tipo = TipoMembroClasse::Get,
                        Some(p::SET) => tipo = TipoMembroClasse::Set,
                        _ => tipo = TipoMembroClasse::Acessor,
                    }
                }
            }
        }
        if self.e(Pont::Vezes) && matches!(tipo, TipoMembroClasse::Metodo) {
            self.avancar()?;
            gerador = true;
        }
        let chave = self.chave()?;
        if self.e(Pont::AbreParen) && tipo != TipoMembroClasse::Acessor {
            let construtor = !estatico && self.e_construtor(&chave);
            if construtor && (assincrona || gerador || tipo != TipoMembroClasse::Metodo) {
                return self.erro(pos, "constructor especial (async/gerador/get/set)");
            }
            let f = self.metodo(pos, assincrona, gerador, tipo == TipoMembroClasse::Get, tipo == TipoMembroClasse::Set, construtor, derivada)?;
            let valor = self.no(No::Funcao(f), pos);
            return Ok(MembroClasse { chave, tipo, estatico, valor });
        }
        if assincrona || gerador || matches!(tipo, TipoMembroClasse::Get | TipoMembroClasse::Set) {
            return self.erro_aqui("esperava ( do método");
        }
        if self.e_construtor(&chave) {
            return self.erro(pos, "campo chamado constructor");
        }
        // Campo: `x = valor;` ou `x;` (o valor vira função sem parâmetros).
        let valor = if self.comer(Pont::Atrib)? {
            let flags = FlagsFuncao { metodo: true, seta: false, parametros_simples: true, ..FlagsFuncao::default() };
            let (id, pai, ctx, rotulos) = self.entrar_funcao(None, flags, pos);
            self.ctx.em_campo = true;
            let ep = self.atual.inicio;
            let r = self.atribuicao_fechada();
            let r = r.map(|e| {
                let ret = self.no(No::Retorne(e), ep);
                let base = self.rascunho.len();
                self.rascunho.push(ret);
                let corpo = self.lista(base);
                self.arv.funcoes[id.0 as usize].corpo = corpo;
            });
            self.sair_funcao(id, pai, ctx, rotulos);
            r?;
            self.no(No::Funcao(id), pos)
        } else {
            NoId::NADA
        };
        self.fim_comando()?;
        let tipo = if tipo == TipoMembroClasse::Acessor { tipo } else { TipoMembroClasse::Campo };
        Ok(MembroClasse { chave, tipo, estatico, valor })
    }

    fn corpo_bloco_estatico(&mut self, id: FuncaoId) -> R<()> {
        self.esperar(Pont::AbreChave)?;
        self.ctx.em_campo = false;
        let base = self.rascunho.len();
        while !self.e(Pont::FechaChave) {
            if self.atual.tipo == Tipo::Fim {
                return self.erro_aqui("esperava } fechando o bloco static");
            }
            let c = self.item()?;
            self.rascunho.push(c);
        }
        self.avancar()?;
        let corpo = self.lista(base);
        self.arv.funcoes[id.0 as usize].corpo = corpo;
        Ok(())
    }

    // ---------------------------------------------------------------- módulos

    fn item_modulo(&mut self) -> R<NoId> {
        if self.palavra(p::IMPORT) {
            let prox = self.espiar()?;
            if !matches!(prox.tipo, Tipo::Pont(Pont::AbreParen | Pont::Ponto)) {
                return self.import_decl();
            }
        }
        if self.palavra(p::EXPORT) {
            return self.export_decl();
        }
        self.item()
    }

    /// Nome de import/export: átomo ou texto (`"a b" as x`).
    fn nome_modulo(&mut self) -> R<(Option<Atomo>, NoId)> {
        let pos = self.atual.inicio;
        match self.atual.tipo {
            Tipo::Nome => {
                let a = self.atomo().unwrap();
                self.avancar()?;
                Ok((Some(a), NoId::NADA))
            }
            Tipo::Texto => {
                let t = Self::texto_ref(&self.atual);
                self.avancar()?;
                Ok((None, self.no(No::Texto(t), pos)))
            }
            _ => self.erro_aqui("esperava nome"),
        }
    }

    fn fonte_modulo(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        if self.atual.tipo != Tipo::Texto {
            return self.erro_aqui("esperava o caminho do módulo entre aspas");
        }
        let t = Self::texto_ref(&self.atual);
        self.avancar()?;
        let f = self.no(No::Texto(t), pos);
        // Atributos de importação (`with { type: "json" }`): lidos e ignorados por enquanto.
        if (self.palavra(p::WITH) || (self.atomo().is_some_and(|a| self.lex.atomos.texto(a) == "assert") && !self.atual.quebra_antes)) && self.espiar()?.tipo == Tipo::Pont(Pont::AbreChave) {
            self.avancar()?;
            let _ = self.lhs()?;
        }
        Ok(f)
    }

    fn import_decl(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        let base = self.rascunho.len();
        if self.atual.tipo == Tipo::Texto {
            let fonte = self.fonte_modulo()?;
            self.fim_comando()?;
            let itens = self.lista(base);
            return Ok(self.no(No::Import { itens, fonte }, pos));
        }
        // Padrão: `import x from`.
        if self.atual.tipo == Tipo::Nome {
            let ip = self.atual.inicio;
            let local = self.nome_ident()?;
            let i = self.no(No::Importado { nome: Some(p::DEFAULT), nome_texto: NoId::NADA, local, padrao: true }, ip);
            self.rascunho.push(i);
            if !self.comer(Pont::Virgula)? {
                return self.fim_import(base, pos);
            }
        }
        if self.e(Pont::Vezes) {
            let ip = self.atual.inicio;
            self.avancar()?;
            self.esperar_palavra(p::AS)?;
            let local = self.nome_ident()?;
            let i = self.no(No::Importado { nome: None, nome_texto: NoId::NADA, local, padrao: false }, ip);
            self.rascunho.push(i);
        } else {
            self.esperar(Pont::AbreChave)?;
            while !self.e(Pont::FechaChave) {
                let ip = self.atual.inicio;
                let (nome, nome_texto) = self.nome_modulo()?;
                let local = if self.comer_palavra(p::AS)? {
                    self.nome_ident()?
                } else {
                    let Some(n) = nome else { return self.erro(ip, "import de texto precisa de 'as'") };
                    self.checar_vinculo(n, ip)?;
                    n
                };
                let i = self.no(No::Importado { nome, nome_texto, local, padrao: false }, ip);
                self.rascunho.push(i);
                if !self.comer(Pont::Virgula)? {
                    break;
                }
            }
            self.esperar(Pont::FechaChave)?;
        }
        self.fim_import(base, pos)
    }

    fn fim_import(&mut self, base: usize, pos: u32) -> R<NoId> {
        self.esperar_palavra(p::FROM)?;
        let fonte = self.fonte_modulo()?;
        self.fim_comando()?;
        let itens = self.lista(base);
        Ok(self.no(No::Import { itens, fonte }, pos))
    }

    fn export_decl(&mut self) -> R<NoId> {
        let pos = self.atual.inicio;
        self.avancar()?;
        if self.e(Pont::Vezes) {
            self.avancar()?;
            let (como, como_texto) = if self.comer_palavra(p::AS)? { self.nome_modulo()? } else { (None, NoId::NADA) };
            self.esperar_palavra(p::FROM)?;
            let fonte = self.fonte_modulo()?;
            self.fim_comando()?;
            return Ok(self.no(No::ExportTudo { como, como_texto, fonte }, pos));
        }
        if self.e(Pont::AbreChave) {
            self.avancar()?;
            let base = self.rascunho.len();
            while !self.e(Pont::FechaChave) {
                let ip = self.atual.inicio;
                let (local, local_texto) = self.nome_modulo()?;
                let (exportado, exportado_texto) = if self.comer_palavra(p::AS)? { self.nome_modulo()? } else { (local, local_texto) };
                let i = self.no(No::Exportado { local, local_texto, exportado, exportado_texto }, ip);
                self.rascunho.push(i);
                if !self.comer(Pont::Virgula)? {
                    break;
                }
            }
            self.esperar(Pont::FechaChave)?;
            let fonte = if self.comer_palavra(p::FROM)? { self.fonte_modulo()? } else { NoId::NADA };
            self.fim_comando()?;
            let itens = self.lista(base);
            return Ok(self.no(No::ExportLista { itens, fonte }, pos));
        }
        if self.comer_palavra(p::DEFAULT)? {
            let dp = self.atual.inicio;
            let d = if self.palavra(p::FUNCTION) {
                self.decl_funcao(false, dp, true)?
            } else if self.palavra(p::ASYNC) && {
                let prox = self.espiar()?;
                prox.dado == Dado::Atomo(p::FUNCTION) && !prox.quebra_antes
            } {
                self.avancar()?;
                self.decl_funcao(true, dp, true)?
            } else if self.palavra(p::CLASS) {
                let c = self.classe(false)?;
                self.no(No::DeclClasse(c), dp)
            } else {
                let e = self.atribuicao_dentro_pub()?;
                self.fim_comando()?;
                e
            };
            return Ok(self.no(No::ExportPadrao(d), pos));
        }
        let d = self.item()?;
        if !matches!(self.arv.no(d), No::Var { .. } | No::DeclFuncao(_) | No::DeclClasse(_)) {
            return self.erro(pos, "export de algo que não é declaração");
        }
        Ok(self.no(No::ExportDecl(d), pos))
    }
}

#[cfg(test)]
mod testes {
    use crate::arvore::No;
    use crate::sintaxe::{analisar_modulo, analisar_script};

    fn ok(src: &str) {
        if let Err(e) = analisar_script(src) {
            panic!("{src:?}: {} em {}", e.mensagem, e.posicao);
        }
    }

    fn falha(src: &str) {
        assert!(analisar_script(src).is_err(), "devia falhar: {src:?}");
    }

    #[test]
    fn expressoes_e_comandos() {
        ok("var a = 1, b = a + 2 * 3; a = b ? 1 : 2; a += 1; a++; --b;");
        ok("x = function(a, b) { return a + b }; y = (a, b) => a * b; z = async x => await x;");
        ok("for (var i = 0; i < 10; i++) { if (i) continue; else break; }");
        ok("for (const [k, v] of m) {} for (k in o) ; for (let {a, b = 2} of x) {}");
        ok("[a, b] = [b, a]; ({a, b: {c = 1}} = o); ({ a = 1 } = {});");
        ok("label: for (;;) { break label; }");
        ok("switch (x) { case 1: f(); break; default: g() }");
        ok("try { a() } catch { } finally { b() } try {} catch ({ message }) {}");
        ok("class A extends B { #x = 1; static y; constructor() { super(); this.#x++ } get z() { return #x in this } static { this.y = 1 } }");
        ok("a?.b?.[c]?.(d); a ?? b; x ||= 1; y &&= 2; z ??= 3; 2 ** 3 ** 2;");
        ok("var t = `a${b}c${`d${e}`}f`; tag`x${1}`; /re/g.test(s); a = b / c / d;");
        ok("let x = 1\nlet y = 2\nreturnValue()\n++z");
        ok("var o = { get a() { return 1 }, set a(v) {}, async *g() { yield 1 }, [k]: 2, 'q': 3, 4: 5, ...r };");
        ok("function* g() { yield; yield* h(); const x = yield 1; }");
        ok("async function f() { for await (const x of y) {} await 1 }");
        ok("new A; new A.B(1); new (f())(); function f() { return new.target }");
        ok("if (a) function f() {}");
        ok("let\nx = 1");
        ok("a = { b: 1 }\n(c)");
        ok("var x = 010; (function(){ 'use strict'; return 1 })");
        falha("(function(){ 'use strict'; var x = 010 })");
    }

    #[test]
    fn erros() {
        falha("var = 1");
        falha("({a = 1})");
        falha("a ?? b || c");
        falha("-a ** 2");
        falha("'use strict'; with (a) {}");
        falha("break;");
        falha("new.target");
        falha("return 1");
        falha("class { }");
        falha("for (let a, b of x) {}");
        falha("const a;");
        falha("x => {} ()");
    }

    #[test]
    fn modulo() {
        let (a, _) = analisar_modulo("import x, { y as z, 'w w' as v } from './m.js'; import * as ns from 'n'; export const q = 1; export default class {} export { q as r }; export * from 'x'; await f();").unwrap();
        assert_eq!(a.lista(a.corpo).len(), 7);
        assert!(matches!(a.no(a.lista(a.corpo)[0]), No::Import { .. }));
    }

    #[test]
    fn flags_de_funcao() {
        let (a, _) = analisar_script("function f() { return () => this.x + arguments[0] } function g(a = 1) { eval('') }").unwrap();
        assert!(a.funcoes[0].flags.usa_this && a.funcoes[0].flags.usa_arguments);
        assert!(!a.funcoes[0].flags.eval_direto && a.funcoes[2].flags.eval_direto);
        assert!(!a.funcoes[2].flags.parametros_simples);
    }
}
