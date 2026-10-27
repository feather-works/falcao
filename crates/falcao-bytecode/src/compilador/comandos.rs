//! Comandos e classes.

use super::{Alvo, Compilador, Destino, Saida, R};
use crate::emissor::{Arg, Rotulo};
use crate::funcao::{Constante, Excecao};
use crate::instrucoes::Op;
use falcao_sintaxe::arvore::*;
use falcao_sintaxe::atomos::Atomo;
use falcao_sintaxe::{Ref, TipoEscopo};

impl Compilador<'_> {
    pub(crate) fn lista_comandos(&mut self, l: Lista) -> R<()> {
        for &c in self.arv.lista(l).to_vec().iter() {
            self.comando(c)?;
        }
        Ok(())
    }

    /// Entra no escopo do nó, se ele abre um.
    fn com_escopo_do_no(&mut self, n: NoId, f: impl FnOnce(&mut Self) -> R<()>) -> R<()> {
        match self.esc.escopo_do_no.get(&n.0).copied() {
            Some(e) => {
                let base = self.entrar_escopo(e);
                let r = f(self);
                self.sair_escopo(e, base);
                r
            }
            None => f(self),
        }
    }

    pub(crate) fn comando(&mut self, n: NoId) -> R<()> {
        self.comando_rotulado(n, Vec::new())
    }

    fn comando_rotulado(&mut self, n: NoId, rotulos: Vec<Atomo>) -> R<()> {
        let base_regs = self.f.topo;
        match *self.arv.no(n) {
            No::Bloco(l) => self.com_escopo_do_no(n, |c| {
                c.icar_funcoes(l)?;
                c.lista_comandos(l)
            })?,
            No::Vazio | No::Depurador => {}
            No::Expressao(e) => {
                self.posicao(n);
                self.expr(e)?;
            }
            No::Var { tipo, decls } => self.declaracao(tipo, decls)?,
            No::Se { teste, sim, nao } => {
                let senao = self.rotulo();
                self.expr(teste)?;
                self.salta(Op::SaltaSeFalso, senao);
                self.comando(sim)?;
                if nao.existe() {
                    let fim = self.rotulo();
                    self.salta(Op::Salta, fim);
                    self.por(senao);
                    self.comando(nao)?;
                    self.por(fim);
                } else {
                    self.por(senao);
                }
            }
            No::Para { inicio, teste, passo, corpo } => self.com_escopo_do_no(n, |c| {
                if inicio.existe() {
                    match *c.arv.no(inicio) {
                        No::Var { tipo, decls } => c.declaracao(tipo, decls)?,
                        _ => c.expr(inicio)?,
                    }
                }
                let renova = c.esc.escopo_do_no.get(&n.0).is_some_and(|&e| c.empurra(e));
                let volta = c.rotulo();
                let continua = c.rotulo();
                let fim = c.rotulo();
                c.por(volta);
                if teste.existe() {
                    c.expr(teste)?;
                    c.salta(Op::SaltaSeFalso, fim);
                }
                c.corpo_de_laco(corpo, rotulos, fim, continua)?;
                c.por(continua);
                if renova {
                    c.op0(Op::RenovaContexto);
                }
                if passo.existe() {
                    c.expr(passo)?;
                }
                c.salta(Op::SaltaAtras, volta);
                c.por(fim);
                Ok(())
            })?,
            No::ParaEm { esq, dir, corpo } => self.com_escopo_do_no(n, |c| {
                let st = c.temp();
                c.expr(dir)?;
                c.op(Op::PreparaForIn, &[Arg::U(st)]);
                let volta = c.rotulo();
                let fim = c.rotulo();
                c.por(volta);
                c.op(Op::ProximoForIn, &[Arg::U(st), Arg::Para(fim)]);
                c.alvo_de_laco(n, esq)?;
                c.corpo_de_laco(corpo, rotulos, fim, volta)?;
                c.salta(Op::SaltaAtras, volta);
                c.por(fim);
                Ok(())
            })?,
            No::ParaDe { esq, dir, corpo, aguarda } => self.com_escopo_do_no(n, |c| {
                let it = c.temp();
                c.expr(dir)?;
                // shortcut: `for await` usa o mesmo Proximo; o runtime espera cada promessa.
                c.op(Op::ObtemIterador, &[Arg::U(it), Arg::U(u32::from(aguarda))]);
                let volta = c.rotulo();
                let fim = c.rotulo();
                let fecha = c.rotulo();
                let ini = c.f.em.aqui();
                c.f.saidas.push(Saida::Iterador(it));
                c.por(volta);
                c.op(Op::Proximo, &[Arg::U(it), Arg::Para(fim)]);
                c.alvo_de_laco(n, esq)?;
                c.corpo_de_laco(corpo, rotulos, fim, volta)?;
                c.salta(Op::SaltaAtras, volta);
                c.f.saidas.pop();
                // Exceção dentro do laço: fecha o iterador e relança.
                let fim_faixa = c.f.em.aqui();
                let contextos = c.f.contextos;
                c.por(fecha);
                let exc = c.temp();
                c.op(Op::Guarda, &[Arg::U(exc)]);
                c.op(Op::FechaIterador, &[Arg::U(it)]);
                c.op(Op::Relanca, &[Arg::U(exc)]);
                let alvo = c.f.em.endereco(fecha);
                c.f.excecoes.push(Excecao { inicio: ini, fim: fim_faixa, alvo, contextos });
                c.por(fim);
                Ok(())
            })?,
            No::Enquanto { teste, corpo } => {
                let volta = self.rotulo();
                let fim = self.rotulo();
                self.por(volta);
                self.expr(teste)?;
                self.salta(Op::SaltaSeFalso, fim);
                self.corpo_de_laco(corpo, rotulos, fim, volta)?;
                self.salta(Op::SaltaAtras, volta);
                self.por(fim);
            }
            No::FacaEnquanto { corpo, teste } => {
                let volta = self.rotulo();
                let continua = self.rotulo();
                let fim = self.rotulo();
                self.por(volta);
                self.corpo_de_laco(corpo, rotulos, fim, continua)?;
                self.por(continua);
                self.expr(teste)?;
                self.salta(Op::SaltaSeFalso, fim);
                self.salta(Op::SaltaAtras, volta);
                self.por(fim);
            }
            No::Continue(r) | No::Break(r) => {
                let continua = matches!(self.arv.no(n), No::Continue(_));
                let alvo = self.f.alvos.iter().rev().find(|a| match r {
                    Some(r) => a.rotulos.contains(&r) && (!continua || a.continua.is_some()),
                    None => !continua || a.continua.is_some(),
                });
                let Some(a) = alvo else { return self.erro(n, "break/continue sem alvo") };
                let (nivel, destino) = (a.nivel, if continua { a.continua.unwrap() } else { a.quebra });
                self.sair_para(nivel, Destino::Para(destino));
            }
            No::Retorne(arg) => {
                if arg.existe() {
                    self.expr(arg)?;
                } else {
                    self.op0(Op::CarregaUndef);
                }
                self.posicao(n);
                self.sair_para(0, Destino::Retorno);
            }
            No::Com { obj, corpo } => {
                self.expr(obj)?;
                self.op0(Op::EmpurraCom);
                self.f.contextos += 1;
                self.f.saidas.push(Saida::Contexto);
                let antes = self.f.escopo;
                if let Some(&e) = self.esc.escopo_do_no.get(&n.0) {
                    self.f.escopo = e;
                }
                self.comando(corpo)?;
                self.f.escopo = antes;
                self.op0(Op::SaiContexto);
                self.f.contextos -= 1;
                self.f.saidas.pop();
            }
            No::Escolha { disc, casos } => {
                let rd = self.expr_reg(disc)?;
                self.com_escopo_do_no(n, |c| c.escolha(rd, casos, rotulos))?;
            }
            No::Rotulo { rotulo, corpo } => {
                let mut rs = rotulos;
                rs.push(rotulo);
                if matches!(self.arv.no(corpo), No::Para { .. } | No::ParaEm { .. } | No::ParaDe { .. } | No::Enquanto { .. } | No::FacaEnquanto { .. } | No::Rotulo { .. }) {
                    self.comando_rotulado(corpo, rs)?;
                } else {
                    let fim = self.rotulo();
                    let nivel = self.f.saidas.len();
                    self.f.alvos.push(Alvo { rotulos: rs, quebra: fim, continua: None, nivel });
                    let r = self.comando(corpo);
                    self.f.alvos.pop();
                    r?;
                    self.por(fim);
                }
            }
            No::Lance(x) => {
                self.expr(x)?;
                self.posicao(n);
                self.op0(Op::Lanca);
            }
            No::Tente { bloco, param, captura, finalmente } => self.tente(n, bloco, param, captura, finalmente)?,
            No::DeclFuncao(_) => {
                // Já criada na entrada do escopo. Anexo B: a função de bloco vira `var` aqui.
                if let Some(&vv) = self.esc.anexo_b.get(&n.0) {
                    if let Some(Ref::Vinculo(v)) = self.esc.referencia(n) {
                        let nome = self.esc.vinculo(v).nome;
                        self.ler_vinculo(v, nome);
                        if self.local(vv).is_some() {
                            self.gravar_local(vv, false);
                        } else {
                            let k = self.k_nome(nome);
                            self.op(Op::GravaGlobal, &[Arg::U(k), Arg::Cache]);
                        }
                    }
                }
            }
            No::DeclClasse(c) => {
                self.classe(c, None)?;
                match (self.esc.referencia(n), self.arv.classe(c).nome) {
                    (Some(Ref::Vinculo(v)), _) => {
                        self.gravar_local(v, true);
                        self.iniciado(v);
                    }
                    (_, Some(a)) => {
                        let k = self.k_nome(a);
                        self.op(Op::IniciaGlobal, &[Arg::U(k)]);
                    }
                    _ => {}
                }
            }
            No::ExportDecl(d) => self.comando(d)?,
            // Ligação feita no topo (`importacoes`/`exportacoes`); nada roda aqui.
            No::Import { .. } | No::ExportLista { .. } | No::ExportTudo { .. } => {}
            No::ExportPadrao(d) => match *self.arv.no(d) {
                No::DeclFuncao(_) => {}
                No::DeclClasse(c) if self.arv.classe(c).nome.is_some() => self.comando(d)?,
                No::DeclClasse(c) => {
                    self.classe(c, None)?;
                    self.op0(Op::GuardaPadrao);
                }
                _ => {
                    self.expr(d)?;
                    self.op0(Op::GuardaPadrao);
                }
            },
            _ => return self.erro(n, "comando que o gerador não conhece"),
        }
        self.liberar(base_regs);
        Ok(())
    }

    fn declaracao(&mut self, tipo: TipoVar, decls: Lista) -> R<()> {
        for &d in self.arv.lista(decls).to_vec().iter() {
            let No::Declarador { alvo, valor } = *self.arv.no(d) else { continue };
            self.posicao(d);
            if valor.existe() {
                if let No::Nome(a) = *self.arv.no(alvo) {
                    let u: Vec<u16> = self.lex.atomos.texto(a).encode_utf16().collect();
                    self.expr_nomeada(valor, u)?;
                } else {
                    self.expr(valor)?;
                }
                self.atribuir_padrao(alvo, true)?;
            } else if tipo != TipoVar::Var {
                self.op0(Op::CarregaUndef);
                self.atribuir_padrao(alvo, true)?;
            }
        }
        Ok(())
    }

    /// Grava o valor do acc no lado esquerdo de for-in/of (declaração ou alvo).
    fn alvo_de_laco(&mut self, laco: NoId, esq: NoId) -> R<()> {
        // Nova volta com `let` capturado: contexto novo para as funções desta volta.
        if let Some(&e) = self.esc.escopo_do_no.get(&laco.0) {
            if self.empurra(e) && self.esc.escopo(e).tipo == TipoEscopo::Para {
                let t = self.temp();
                self.op(Op::Guarda, &[Arg::U(t)]);
                self.op0(Op::RenovaContexto);
                self.op(Op::Carrega, &[Arg::U(t)]);
                self.liberar(t);
            }
        }
        match *self.arv.no(esq) {
            No::Var { decls, .. } => {
                let d = self.arv.lista(decls)[0];
                let No::Declarador { alvo, .. } = *self.arv.no(d) else { unreachable!() };
                self.atribuir_padrao(alvo, true)
            }
            _ => self.atribuir_padrao(esq, false),
        }
    }

    fn corpo_de_laco(&mut self, corpo: NoId, rotulos: Vec<Atomo>, quebra: Rotulo, continua: Rotulo) -> R<()> {
        let nivel = self.f.saidas.len();
        self.f.alvos.push(Alvo { rotulos, quebra, continua: Some(continua), nivel });
        let r = self.comando(corpo);
        self.f.alvos.pop();
        r
    }

    fn escolha(&mut self, rd: u32, casos: Lista, rotulos: Vec<Atomo>) -> R<()> {
        let casos = self.arv.lista(casos).to_vec();
        for &c in &casos {
            if let No::Caso { corpo, .. } = *self.arv.no(c) {
                self.icar_funcoes(corpo)?;
            }
        }
        let fim = self.rotulo();
        let rotulos_casos: Vec<Rotulo> = casos.iter().map(|_| self.rotulo()).collect();
        let mut padrao = None;
        for (i, &c) in casos.iter().enumerate() {
            let No::Caso { teste, .. } = *self.arv.no(c) else { continue };
            if !teste.existe() {
                padrao = Some(rotulos_casos[i]);
                continue;
            }
            self.expr(teste)?;
            self.op(Op::Identico, &[Arg::U(rd)]);
            self.salta(Op::SaltaSeVerdadeiro, rotulos_casos[i]);
        }
        self.salta(Op::Salta, padrao.unwrap_or(fim));
        let nivel = self.f.saidas.len();
        self.f.alvos.push(Alvo { rotulos, quebra: fim, continua: None, nivel });
        for (i, &c) in casos.iter().enumerate() {
            let No::Caso { corpo, .. } = *self.arv.no(c) else { continue };
            self.por(rotulos_casos[i]);
            if let Err(e) = self.lista_comandos(corpo) {
                self.f.alvos.pop();
                return Err(e);
            }
        }
        self.f.alvos.pop();
        self.por(fim);
        Ok(())
    }

    /// `try`: faixas na tabela de exceções; `finally` com o tipo de saída num registrador
    /// (0 normal, 1 exceção, 2+ saltos e `return` que passam por ele).
    fn tente(&mut self, n: NoId, bloco: NoId, param: NoId, captura: NoId, finalmente: NoId) -> R<()> {
        let tem_finally = finalmente.existe();
        let (tipo, valor) = if tem_finally { (self.temp(), self.temp()) } else { (0, 0) };
        let entrada_fin = self.rotulo();
        if tem_finally {
            self.f.saidas.push(Saida::Finally { tipo, valor, entrada: entrada_fin, saltos: Vec::new() });
        }
        let contextos = self.f.contextos;
        let ini = self.f.em.aqui();
        let depois = self.rotulo();
        self.comando(bloco)?;
        let fim_try = self.f.em.aqui();
        self.salta(Op::Salta, depois);
        if captura.existe() {
            let tratador = self.f.em.aqui();
            self.f.excecoes.push(Excecao { inicio: ini, fim: fim_try, alvo: tratador, contextos });
            // A exceção chega no acc; entrar no escopo do catch inicia vínculos e troca o acc.
            let exc = self.temp();
            self.op(Op::Guarda, &[Arg::U(exc)]);
            self.com_escopo_do_no(n, |c| {
                if param.existe() {
                    c.op(Op::Carrega, &[Arg::U(exc)]);
                    c.atribuir_padrao(param, true)?;
                }
                c.comando(captura)
            })?;
            self.salta(Op::Salta, depois);
        }
        let fim_protegido = self.f.em.aqui();
        self.por(depois);
        if !tem_finally {
            return Ok(());
        }
        // Saída normal.
        self.op(Op::CarregaInt, &[Arg::I(0)]);
        self.op(Op::Guarda, &[Arg::U(tipo)]);
        let normal = self.rotulo();
        self.salta(Op::Salta, normal);
        // Exceção no try ou no catch.
        let tratador = self.f.em.aqui();
        self.f.excecoes.push(Excecao { inicio: ini, fim: fim_protegido, alvo: tratador, contextos });
        self.op(Op::Guarda, &[Arg::U(valor)]);
        self.op(Op::CarregaInt, &[Arg::I(1)]);
        self.op(Op::Guarda, &[Arg::U(tipo)]);
        self.por(normal);
        self.por(entrada_fin);
        let Some(Saida::Finally { saltos, .. }) = self.f.saidas.pop() else { unreachable!() };
        let nivel = self.f.saidas.len();
        self.comando(finalmente)?;
        // Despacho: relança, continua o salto pendente, ou segue.
        let segue = self.rotulo();
        let nao_lanca = self.rotulo();
        self.op(Op::CarregaInt, &[Arg::I(1)]);
        self.op(Op::Identico, &[Arg::U(tipo)]);
        self.salta(Op::SaltaSeFalso, nao_lanca);
        self.op(Op::Relanca, &[Arg::U(valor)]);
        self.por(nao_lanca);
        for (id, destino) in saltos {
            let prox = self.rotulo();
            self.op(Op::CarregaInt, &[Arg::I(id as i32)]);
            self.op(Op::Identico, &[Arg::U(tipo)]);
            self.salta(Op::SaltaSeFalso, prox);
            if destino == Destino::Retorno {
                self.op(Op::Carrega, &[Arg::U(valor)]);
            }
            self.sair_para(nivel, destino);
            self.por(prox);
        }
        self.salta(Op::Salta, segue);
        self.por(segue);
        Ok(())
    }

    // ---------------------------------------------------------------- classes

    /// acc = a classe `c` (o construtor). `nome`: nome inferido, se a classe não tem.
    pub(crate) fn classe(&mut self, c: ClasseId, nome: Option<u32>) -> R<()> {
        let classe = self.arv.classe(c).clone();
        let base = self.f.topo;
        let escopo = self.esc.escopo_da_classe[c.0 as usize];
        let regs = self.entrar_escopo(escopo);
        let rh = self.temp();
        if classe.heranca.existe() {
            self.expr(classe.heranca)?;
        } else {
            self.op0(Op::CarregaBuraco);
        }
        self.op(Op::Guarda, &[Arg::U(rh)]);
        let nome_t = classe.nome.map(|a| self.texto_atomo(a)).or(nome);
        let construtor = match classe.construtor {
            Some(f) => {
                let k = self.k_funcao(f, nome_t);
                let Constante::Funcao(i) = self.f.constantes[k as usize] else { unreachable!() };
                i
            }
            None => u32::MAX,
        };
        let k = self.k_unica(Constante::Classe { construtor, derivada: classe.heranca.existe(), nome: nome_t });
        self.op(Op::CriaClasse, &[Arg::U(k), Arg::U(rh)]);
        let rf = self.temp();
        self.op(Op::Guarda, &[Arg::U(rf)]);
        // O nome da classe dentro dela.
        if classe.nome.is_some() {
            if let Some(&v) = self.esc.escopo(escopo).vinculos.first() {
                self.gravar_local(v, true);
                self.iniciado(v);
            }
        }
        let rp = self.temp();
        let kp = self.k_nome_texto("prototype".encode_utf16().collect());
        self.op(Op::LeProp, &[Arg::U(rf), Arg::U(kp), Arg::Cache]);
        self.op(Op::Guarda, &[Arg::U(rp)]);
        for m in &classe.membros {
            let marca = self.f.topo;
            let alvo = if m.estatico { rf } else { rp };
            match m.tipo {
                TipoMembroClasse::Metodo | TipoMembroClasse::Get | TipoMembroClasse::Set => {
                    let rk = self.chave_reg(m.chave)?;
                    self.metodo_valor(m.valor, alvo)?;
                    match m.tipo {
                        TipoMembroClasse::Metodo => self.op(Op::DefMetodo, &[Arg::U(alvo), Arg::U(rk)]),
                        TipoMembroClasse::Get => self.op(Op::DefAcessor, &[Arg::U(alvo), Arg::U(rk), Arg::U(2)]),
                        _ => self.op(Op::DefAcessor, &[Arg::U(alvo), Arg::U(rk), Arg::U(3)]),
                    }
                }
                TipoMembroClasse::Campo | TipoMembroClasse::Acessor => {
                    let rk = self.chave_reg(m.chave)?;
                    if m.valor.existe() {
                        self.metodo_valor(m.valor, alvo)?;
                    } else {
                        self.op0(Op::CarregaUndef);
                    }
                    if m.estatico {
                        // Campo estático: roda o inicializador agora, com this = classe.
                        let bloco = self.temps(2);
                        self.op(Op::Guarda, &[Arg::U(bloco)]);
                        self.op(Op::Move, &[Arg::U(bloco + 1), Arg::U(rf)]);
                        let pronto = self.rotulo();
                        self.op(Op::Carrega, &[Arg::U(bloco)]);
                        self.salta(Op::SaltaSeIndef, pronto);
                        self.op(Op::Chama, &[Arg::U(bloco), Arg::U(bloco + 1), Arg::U(0), Arg::Cache]);
                        self.por(pronto);
                        self.op(Op::DefItem, &[Arg::U(rf), Arg::U(rk)]);
                    } else {
                        self.op(Op::DefCampo, &[Arg::U(rf), Arg::U(rk)]);
                    }
                }
                TipoMembroClasse::BlocoEstatico => {
                    let bloco = self.temps(2);
                    self.metodo_valor(m.valor, rf)?;
                    self.op(Op::Guarda, &[Arg::U(bloco)]);
                    self.op(Op::Move, &[Arg::U(bloco + 1), Arg::U(rf)]);
                    self.op(Op::Chama, &[Arg::U(bloco), Arg::U(bloco + 1), Arg::U(0), Arg::Cache]);
                }
            }
            self.liberar(marca);
        }
        self.op(Op::Carrega, &[Arg::U(rf)]);
        self.sair_escopo(escopo, regs);
        self.liberar(base);
        Ok(())
    }
}
