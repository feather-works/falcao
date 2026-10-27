//! Padrões (desestruturação): o valor está no acc e vai para os alvos do padrão.

use super::{Compilador, R};
use crate::emissor::Arg;
use crate::instrucoes::Op;
use falcao_sintaxe::arvore::*;

impl Compilador<'_> {
    /// Grava o valor do acc no padrão `n`. `declarando`: `let/const/var`/parâmetro.
    pub(crate) fn atribuir_padrao(&mut self, n: NoId, declarando: bool) -> R<()> {
        match *self.arv.no(n) {
            No::Nome(a) => self.gravar_nome(n, a, declarando),
            No::Paren(x) => return self.atribuir_padrao(x, declarando),
            No::PadraoComValor { alvo, valor } => {
                let tem = self.rotulo();
                self.salta(Op::SaltaSeNaoIndef, tem);
                if let No::Nome(a) = *self.arv.no(alvo) {
                    let u: Vec<u16> = self.lex.atomos.texto(a).encode_utf16().collect();
                    self.expr_nomeada(valor, u)?;
                } else {
                    self.expr(valor)?;
                }
                self.por(tem);
                return self.atribuir_padrao(alvo, declarando);
            }
            No::PadraoArray(l) => return self.padrao_array(l, declarando),
            No::PadraoObjeto(l) => return self.padrao_objeto(l, declarando),
            No::Membro { .. } | No::Indice { .. } | No::MembroPrivado { .. } => {
                let base = self.f.topo;
                let v = self.temp();
                self.op(Op::Guarda, &[Arg::U(v)]);
                let lugar = self.lugar(n)?;
                self.op(Op::Carrega, &[Arg::U(v)]);
                self.gravar_lugar(lugar);
                self.liberar(base);
            }
            _ => return self.erro(n, "alvo de desestruturação inválido"),
        }
        Ok(())
    }

    fn padrao_array(&mut self, l: Lista, declarando: bool) -> R<()> {
        let base = self.f.topo;
        let it = self.temp();
        let acabou = self.temp();
        // shortcut: exceção no meio da desestruturação não fecha o iterador (só o fim normal).
        self.op(Op::ObtemIterador, &[Arg::U(it), Arg::U(0)]);
        self.op0(Op::CarregaFalso);
        self.op(Op::Guarda, &[Arg::U(acabou)]);
        for &item in self.arv.lista(l).to_vec().iter() {
            if let No::PadraoResto(x) = *self.arv.no(item) {
                let arr = self.temp();
                self.op(Op::CriaArray, &[Arg::U(0)]);
                self.op(Op::Guarda, &[Arg::U(arr)]);
                let fim = self.rotulo();
                self.op(Op::Carrega, &[Arg::U(acabou)]);
                self.salta(Op::SaltaSeVerdadeiro, fim);
                let volta = self.rotulo();
                self.por(volta);
                self.op(Op::Proximo, &[Arg::U(it), Arg::Para(fim)]);
                self.op(Op::Empurra, &[Arg::U(arr)]);
                self.salta(Op::SaltaAtras, volta);
                self.por(fim);
                self.op0(Op::CarregaVerdadeiro);
                self.op(Op::Guarda, &[Arg::U(acabou)]);
                self.op(Op::Carrega, &[Arg::U(arr)]);
                self.atribuir_padrao(x, declarando)?;
                continue;
            }
            // acc = próximo valor, ou undefined se o iterador acabou.
            let indef = self.rotulo();
            let ok = self.rotulo();
            let fim_it = self.rotulo();
            self.op(Op::Carrega, &[Arg::U(acabou)]);
            self.salta(Op::SaltaSeVerdadeiro, indef);
            self.op(Op::Proximo, &[Arg::U(it), Arg::Para(fim_it)]);
            self.salta(Op::Salta, ok);
            self.por(fim_it);
            self.op0(Op::CarregaVerdadeiro);
            self.op(Op::Guarda, &[Arg::U(acabou)]);
            self.por(indef);
            self.op0(Op::CarregaUndef);
            self.por(ok);
            if !matches!(self.arv.no(item), No::Buraco) {
                self.atribuir_padrao(item, declarando)?;
            }
        }
        let fechado = self.rotulo();
        self.op(Op::Carrega, &[Arg::U(acabou)]);
        self.salta(Op::SaltaSeVerdadeiro, fechado);
        self.op(Op::FechaIterador, &[Arg::U(it)]);
        self.por(fechado);
        self.liberar(base);
        Ok(())
    }

    fn padrao_objeto(&mut self, l: Lista, declarando: bool) -> R<()> {
        let base = self.f.topo;
        let itens = self.arv.lista(l).to_vec();
        let tem_resto = itens.iter().any(|&i| matches!(self.arv.no(i), No::PadraoResto(_)));
        let src = self.temp();
        self.op0(Op::ParaObjeto);
        self.op(Op::Guarda, &[Arg::U(src)]);
        let n_chaves = itens.len() as u32 - u32::from(tem_resto);
        let chaves = if tem_resto { self.temps(n_chaves) } else { 0 };
        for (i, &item) in itens.iter().enumerate() {
            let marca = self.f.topo;
            match *self.arv.no(item) {
                No::Propriedade { chave, valor, .. } => {
                    match (self.chave_fixa(chave), tem_resto) {
                        (Some(u), false) => {
                            let k = self.k_nome_texto(u);
                            self.op(Op::LeProp, &[Arg::U(src), Arg::U(k), Arg::Cache]);
                        }
                        _ => {
                            let rk = self.chave_reg(chave)?;
                            if tem_resto {
                                self.op(Op::Carrega, &[Arg::U(rk)]);
                                self.op(Op::Guarda, &[Arg::U(chaves + i as u32)]);
                            }
                            self.op(Op::Carrega, &[Arg::U(rk)]);
                            self.op(Op::LeItem, &[Arg::U(src), Arg::Cache]);
                        }
                    }
                    self.atribuir_padrao(valor, declarando)?;
                }
                No::PadraoResto(x) => {
                    self.op(Op::RestoObjeto, &[Arg::U(src), Arg::U(chaves), Arg::U(n_chaves)]);
                    self.atribuir_padrao(x, declarando)?;
                }
                _ => return self.erro(item, "item de padrão de objeto inválido"),
            }
            self.liberar(marca);
        }
        self.liberar(base);
        Ok(())
    }
}
