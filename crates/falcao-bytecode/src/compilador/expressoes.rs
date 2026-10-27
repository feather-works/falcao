//! Expressões: cada uma deixa o resultado no acumulador.

use super::{Compilador, R};
use crate::emissor::{Arg, Rotulo};
use crate::funcao::Constante;
use crate::instrucoes::Op;
use falcao_sintaxe::arvore::*;
use falcao_sintaxe::atomos::{palavras as p, Atomo};
use falcao_sintaxe::Ref;

/// Lugar onde se pode gravar (lado esquerdo de atribuição/atualização), já avaliado.
#[derive(Clone, Copy)]
pub(crate) enum Lugar {
    Nome(NoId, Atomo),
    Prop(u32, u32),
    Item(u32, u32),
    Privado(u32, u32),
    Super(u32, u32),
    SuperItem(u32, u32),
}

impl Compilador<'_> {
    /// acc = valor da expressão `n`.
    pub(crate) fn expr(&mut self, n: NoId) -> R<()> {
        match *self.arv.no(n) {
            No::Nome(a) => self.ler_nome(n, a, false),
            No::This => self.op0(Op::CarregaThis),
            No::Nulo => self.op0(Op::CarregaNulo),
            No::Verdadeiro => self.op0(Op::CarregaVerdadeiro),
            No::Falso => self.op0(Op::CarregaFalso),
            No::Numero(x) => self.numero(x),
            No::BigInt(i) => {
                let t = self.texto_u16(self.lex.texto(i).to_vec());
                let k = self.k_unica(Constante::BigInt(t));
                self.op(Op::CarregaConst, &[Arg::U(k)]);
            }
            No::Texto(t) => {
                let u = self.unidades_de(t);
                let k = self.k_texto(u);
                self.op(Op::CarregaConst, &[Arg::U(k)]);
            }
            No::Template { pedacos, exprs } => self.template(pedacos, exprs)?,
            No::TemplateMarcado { tag, quasi } => self.template_marcado(tag, quasi)?,
            No::Regex { corpo, flags } => {
                let c: Vec<u16> = self.lex.fonte()[corpo.0 as usize..corpo.1 as usize].encode_utf16().collect();
                let f: Vec<u16> = self.lex.fonte()[flags.0 as usize..flags.1 as usize].encode_utf16().collect();
                let (tc, tf) = (self.texto_u16(c), self.texto_u16(f));
                let k = self.k_unica(Constante::Regex { corpo: tc, flags: tf });
                let kf = self.k_unica(Constante::Texto(tf));
                self.op(Op::CriaRegex, &[Arg::U(k), Arg::U(kf)]);
            }
            No::Array(l) => self.array(l)?,
            No::Objeto(l) => self.objeto(l)?,
            No::Funcao(f) => self.criar_funcao(f, None),
            No::Classe(c) => self.classe(c, None)?,
            No::Membro { .. } | No::Indice { .. } | No::MembroPrivado { .. } => self.membro(n)?,
            No::Chamada { .. } => self.chamada(n)?,
            No::New { func, args } => {
                self.posicao(n);
                let rf = self.temp();
                self.expr(func)?;
                self.op(Op::Guarda, &[Arg::U(rf)]);
                let (ra, na, espalha) = self.argumentos(args)?;
                self.posicao(n);
                if espalha {
                    self.op(Op::NewEspalhado, &[Arg::U(rf), Arg::U(ra), Arg::U(na)]);
                } else {
                    self.op(Op::New, &[Arg::U(rf), Arg::U(ra), Arg::U(na), Arg::Cache]);
                }
                self.liberar(rf);
            }
            No::CadeiaOpcional(x) => {
                let nulo = self.rotulo();
                let fim = self.rotulo();
                self.opcionais.push(nulo);
                let r = self.expr(x);
                self.opcionais.pop();
                r?;
                self.salta(Op::Salta, fim);
                self.por(nulo);
                self.op0(Op::CarregaUndef);
                self.por(fim);
            }
            No::NewTarget => self.op0(Op::CarregaNewTarget),
            No::ImportMeta => self.op0(Op::ImportMeta),
            No::ImportDinamico { arg, .. } => {
                let r = self.temp();
                self.expr(arg)?;
                self.op(Op::Guarda, &[Arg::U(r)]);
                self.op(Op::ImportDinamico, &[Arg::U(r)]);
                self.liberar(r);
            }
            No::Unario { op, arg } => self.unario(n, op, arg)?,
            No::Atualizacao { incremento, prefixo, alvo } => {
                self.posicao(n);
                let base = self.f.topo;
                let lugar = self.lugar(alvo)?;
                self.ler_lugar(lugar);
                self.op0(Op::ParaNumerico);
                let velho = if prefixo { None } else { Some(self.temp()) };
                if let Some(v) = velho {
                    self.op(Op::Guarda, &[Arg::U(v)]);
                }
                self.op(if incremento { Op::Incrementa } else { Op::Decrementa }, &[Arg::Cache]);
                self.gravar_lugar(lugar);
                if let Some(v) = velho {
                    self.op(Op::Carrega, &[Arg::U(v)]);
                }
                self.liberar(base);
            }
            No::Binario { op, esq, dir } => self.binario(n, op, esq, dir)?,
            No::Logico { op, esq, dir } => {
                let fim = self.rotulo();
                self.expr(esq)?;
                self.salto_logico(op, fim);
                self.expr(dir)?;
                self.por(fim);
            }
            No::Condicional { teste, sim, nao } => {
                let senao = self.rotulo();
                let fim = self.rotulo();
                self.expr(teste)?;
                self.salta(Op::SaltaSeFalso, senao);
                self.expr(sim)?;
                self.salta(Op::Salta, fim);
                self.por(senao);
                self.expr(nao)?;
                self.por(fim);
            }
            No::Atribuicao { op, alvo, valor } => self.atribuicao(n, op, alvo, valor)?,
            No::Sequencia(l) => {
                for &x in self.arv.lista(l).to_vec().iter() {
                    self.expr(x)?;
                }
            }
            No::Yield { arg, delegado } => {
                if arg.existe() {
                    self.expr(arg)?;
                } else {
                    self.op0(Op::CarregaUndef);
                }
                if delegado {
                    let it = self.temp();
                    self.op(Op::ObtemIterador, &[Arg::U(it), Arg::U(u32::from(self.f.flags.assincrona))]);
                    let fim = self.rotulo();
                    self.op(Op::Delega, &[Arg::U(it), Arg::Para(fim)]);
                    self.por(fim);
                    self.liberar(it);
                } else {
                    if self.f.flags.assincrona {
                        self.op0(Op::Aguarda);
                    }
                    self.op0(Op::Suspende);
                }
            }
            No::Await(x) => {
                self.expr(x)?;
                self.posicao(n);
                self.op0(Op::Aguarda);
            }
            No::Paren(x) => self.expr(x)?,
            _ => return self.erro(n, "expressão que o gerador não conhece"),
        }
        Ok(())
    }

    pub(crate) fn numero(&mut self, x: f64) {
        if x.fract() == 0.0 && x.abs() < 1e9 && !(x == 0.0 && x.is_sign_negative()) {
            self.op(Op::CarregaInt, &[Arg::I(x as i32)]);
        } else {
            let k = self.k_numero(x);
            self.op(Op::CarregaConst, &[Arg::U(k)]);
        }
    }

    /// Expressão num registrador temporário novo.
    pub(crate) fn expr_reg(&mut self, n: NoId) -> R<u32> {
        let r = self.temp();
        self.expr(n)?;
        self.op(Op::Guarda, &[Arg::U(r)]);
        Ok(r)
    }

    /// Valor com nome inferido (`const f = function(){}` → `f.name == "f"`).
    pub(crate) fn expr_nomeada(&mut self, n: NoId, nome: Vec<u16>) -> R<()> {
        match *self.arv.no(n) {
            No::Funcao(f) if self.arv.funcao(f).nome.is_none() => {
                let t = self.texto_u16(nome);
                self.criar_funcao(f, Some(t));
                Ok(())
            }
            No::Classe(c) if self.arv.classe(c).nome.is_none() => {
                let t = self.texto_u16(nome);
                self.classe(c, Some(t))
            }
            No::Paren(x) => self.expr_nomeada(x, nome),
            _ => self.expr(n),
        }
    }

    pub(crate) fn criar_funcao(&mut self, f: FuncaoId, nome: Option<u32>) {
        let k = self.k_funcao(f, nome);
        self.op(Op::CriaFuncao, &[Arg::U(k)]);
    }

    fn salto_logico(&mut self, op: OpLogico, fim: Rotulo) {
        let s = match op {
            OpLogico::E => Op::SaltaSeFalso,
            OpLogico::Ou => Op::SaltaSeVerdadeiro,
            OpLogico::Coalesce => Op::SaltaSeNaoNuloOuIndef,
        };
        self.salta(s, fim);
    }

    // ---------------------------------------------------------------- literais

    fn template(&mut self, pedacos: Lista, exprs: Lista) -> R<()> {
        let ps = self.arv.lista(pedacos).to_vec();
        let es = self.arv.lista(exprs).to_vec();
        let acc = self.temp();
        self.pedaco(ps[0])?;
        for (i, &e) in es.iter().enumerate() {
            self.op(Op::Guarda, &[Arg::U(acc)]);
            self.expr(e)?;
            self.op0(Op::ParaTexto);
            self.op(Op::Soma, &[Arg::U(acc), Arg::Cache]);
            let prox = ps[i + 1];
            if let No::PedacoTemplate(j) = *self.arv.no(prox) {
                if self.lex.template(j).cozido.as_ref().is_some_and(|c| c.is_empty()) {
                    continue;
                }
            }
            self.op(Op::Guarda, &[Arg::U(acc)]);
            self.pedaco(prox)?;
            self.op(Op::Soma, &[Arg::U(acc), Arg::Cache]);
        }
        self.liberar(acc);
        Ok(())
    }

    fn pedaco(&mut self, n: NoId) -> R<()> {
        let No::PedacoTemplate(i) = *self.arv.no(n) else { return self.erro(n, "template sem pedaço") };
        let u = self.lex.template(i).cozido.clone().unwrap_or_default();
        let k = self.k_texto(u);
        self.op(Op::CarregaConst, &[Arg::U(k)]);
        Ok(())
    }

    fn template_marcado(&mut self, tag: NoId, quasi: NoId) -> R<()> {
        let No::Template { pedacos, exprs } = *self.arv.no(quasi) else { return self.erro(quasi, "template marcado sem template") };
        let mut cozidos = Vec::new();
        let mut crus = Vec::new();
        for &pn in self.arv.lista(pedacos).to_vec().iter() {
            let No::PedacoTemplate(i) = *self.arv.no(pn) else { continue };
            let parte = self.lex.template(i).clone();
            cozidos.push(parte.cozido.map(|c| self.texto_u16(c)));
            crus.push(self.texto_u16(parte.cru));
        }
        let k = self.k_unica(Constante::Template { cozidos, crus });
        let es = self.arv.lista(exprs).to_vec();
        let base = self.f.topo;
        let (rf, rthis) = self.funcao_e_this(tag)?;
        let ra = self.temps(1 + es.len() as u32);
        debug_assert_eq!(ra, rthis + 1);
        self.op(Op::ObjetoTemplate, &[Arg::U(k)]);
        self.op(Op::Guarda, &[Arg::U(ra)]);
        for (i, &e) in es.iter().enumerate() {
            self.expr(e)?;
            self.op(Op::Guarda, &[Arg::U(ra + 1 + i as u32)]);
        }
        self.op(Op::Chama, &[Arg::U(rf), Arg::U(rthis), Arg::U(1 + es.len() as u32), Arg::Cache]);
        self.liberar(base);
        Ok(())
    }

    fn array(&mut self, l: Lista) -> R<()> {
        let itens = self.arv.lista(l).to_vec();
        let t = self.temp();
        self.op(Op::CriaArray, &[Arg::U(itens.len() as u32)]);
        self.op(Op::Guarda, &[Arg::U(t)]);
        for &x in &itens {
            match *self.arv.no(x) {
                No::Buraco => self.op(Op::EmpurraBuraco, &[Arg::U(t)]),
                No::Espalhar(e) => {
                    self.expr(e)?;
                    self.op(Op::EspalhaArray, &[Arg::U(t)]);
                }
                _ => {
                    self.expr(x)?;
                    self.op(Op::Empurra, &[Arg::U(t)]);
                }
            }
        }
        self.op(Op::Carrega, &[Arg::U(t)]);
        self.liberar(t);
        Ok(())
    }

    /// Texto de uma chave fixa (nome, texto, número inteiro), ou `None` se precisa calcular ou é
    /// índice de array: `{1: x}` / `{"1": x}` guardam no índice 1, o mesmo que `o[1]` lê (o
    /// carregador do browserify do fast.com, `e[3]`).
    pub(crate) fn chave_fixa(&self, c: Chave) -> Option<Vec<u16>> {
        self.texto_da_chave(c).filter(|u| !e_indice(u))
    }

    fn texto_da_chave(&self, c: Chave) -> Option<Vec<u16>> {
        match c {
            Chave::Atomo(a) => Some(self.lex.atomos.texto(a).encode_utf16().collect()),
            Chave::Texto(t) => Some(self.unidades_de(t)),
            Chave::Numero(x) if x.fract() == 0.0 && x >= 0.0 && x < 1e15 => Some(format!("{}", x as u64).encode_utf16().collect()),
            _ => None,
        }
    }

    /// Põe a chave (calculada ou não) num registrador novo, já como chave de propriedade.
    pub(crate) fn chave_reg(&mut self, c: Chave) -> R<u32> {
        let r = self.temp();
        match c {
            Chave::Calculada(e) => self.expr(e)?,
            Chave::Numero(x) => self.numero(x),
            Chave::BigInt(i) => {
                let t = self.texto_u16(self.lex.texto(i).to_vec());
                let k = self.k_unica(Constante::BigInt(t));
                self.op(Op::CarregaConst, &[Arg::U(k)]);
            }
            Chave::Privada(a) => {
                let k = self.k_privado(a);
                self.op(Op::ChavePrivada, &[Arg::U(k)]);
                self.op(Op::Guarda, &[Arg::U(r)]);
                return Ok(r);
            }
            _ => {
                let u = self.texto_da_chave(c).unwrap();
                let k = self.k_texto(u);
                self.op(Op::CarregaConst, &[Arg::U(k)]);
            }
        }
        self.op0(Op::ParaChave);
        self.op(Op::Guarda, &[Arg::U(r)]);
        Ok(r)
    }

    fn objeto(&mut self, l: Lista) -> R<()> {
        let props = self.arv.lista(l).to_vec();
        let t = self.temp();
        self.op(Op::CriaObjeto, &[Arg::U(props.len().min(32) as u32)]);
        self.op(Op::Guarda, &[Arg::U(t)]);
        for &x in &props {
            let base = self.f.topo;
            match *self.arv.no(x) {
                No::Espalhar(e) => {
                    self.expr(e)?;
                    self.op(Op::EspalhaObjeto, &[Arg::U(t)]);
                }
                No::Propriedade { chave, valor, tipo } => match tipo {
                    TipoProp::Normal | TipoProp::Curta => {
                        let fixa = self.chave_fixa(chave);
                        if tipo == TipoProp::Normal && matches!(chave, Chave::Atomo(_) | Chave::Texto(_)) && fixa.as_deref() == Some(&"__proto__".encode_utf16().collect::<Vec<_>>()[..]) {
                            self.expr(valor)?;
                            self.op(Op::DefProto, &[Arg::U(t)]);
                        } else if let Some(u) = fixa {
                            self.expr_nomeada(valor, u.clone())?;
                            let k = self.k_nome_texto(u);
                            self.op(Op::DefProp, &[Arg::U(t), Arg::U(k)]);
                        } else {
                            let rk = self.chave_reg(chave)?;
                            match self.texto_da_chave(chave) {
                                Some(u) => self.expr_nomeada(valor, u)?,
                                None => self.expr(valor)?,
                            }
                            self.op(Op::DefItem, &[Arg::U(t), Arg::U(rk)]);
                        }
                    }
                    TipoProp::Metodo => {
                        let rk = self.chave_reg(chave)?;
                        self.metodo_valor(valor, t)?;
                        self.op(Op::DefItem, &[Arg::U(t), Arg::U(rk)]);
                    }
                    TipoProp::Get | TipoProp::Set => {
                        let rk = self.chave_reg(chave)?;
                        self.metodo_valor(valor, t)?;
                        let n = if tipo == TipoProp::Get { 0 } else { 1 };
                        self.op(Op::DefAcessor, &[Arg::U(t), Arg::U(rk), Arg::U(n)]);
                    }
                },
                _ => return self.erro(x, "item de objeto inválido"),
            }
            self.liberar(base);
        }
        self.op(Op::Carrega, &[Arg::U(t)]);
        self.liberar(t);
        Ok(())
    }

    /// Cria a função de um método e liga a casa (para `super`).
    pub(crate) fn metodo_valor(&mut self, valor: NoId, casa: u32) -> R<()> {
        let No::Funcao(f) = *self.arv.no(valor) else { return self.expr(valor) };
        self.criar_funcao(f, None);
        self.op(Op::DefineCasa, &[Arg::U(casa)]);
        Ok(())
    }

    // ---------------------------------------------------------------- membros

    fn membro(&mut self, n: NoId) -> R<()> {
        self.posicao(n);
        match *self.arv.no(n) {
            No::Membro { obj, prop, opcional } => {
                if matches!(self.arv.no(obj), No::Super) {
                    let t = self.temp();
                    self.op0(Op::CarregaThis);
                    self.op(Op::Guarda, &[Arg::U(t)]);
                    let k = self.k_nome(prop);
                    self.op(Op::LeSuper, &[Arg::U(t), Arg::U(k)]);
                    self.liberar(t);
                    return Ok(());
                }
                self.expr(obj)?;
                self.opcional(opcional);
                let k = self.k_nome(prop);
                self.posicao(n);
                self.op(Op::LePropAcc, &[Arg::U(k), Arg::Cache]);
            }
            No::Indice { obj, prop, opcional } => {
                if matches!(self.arv.no(obj), No::Super) {
                    let base = self.f.topo;
                    let t = self.temp();
                    self.op0(Op::CarregaThis);
                    self.op(Op::Guarda, &[Arg::U(t)]);
                    let rk = self.expr_reg(prop)?;
                    self.op(Op::LeSuperItem, &[Arg::U(t), Arg::U(rk)]);
                    self.liberar(base);
                    return Ok(());
                }
                let r = self.temp();
                self.expr(obj)?;
                self.opcional(opcional);
                self.op(Op::Guarda, &[Arg::U(r)]);
                self.expr(prop)?;
                self.posicao(n);
                self.op(Op::LeItem, &[Arg::U(r), Arg::Cache]);
                self.liberar(r);
            }
            No::MembroPrivado { obj, prop, opcional } => {
                let r = self.temp();
                self.expr(obj)?;
                self.opcional(opcional);
                self.op(Op::Guarda, &[Arg::U(r)]);
                let k = self.k_privado(prop);
                self.op(Op::LePrivado, &[Arg::U(r), Arg::U(k)]);
                self.liberar(r);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    pub(crate) fn k_privado(&mut self, a: Atomo) -> u32 {
        let u: Vec<u16> = format!("#{}", self.lex.atomos.texto(a)).encode_utf16().collect();
        self.k_nome_texto(u)
    }

    /// Depois de avaliar o objeto de um `?.`: se é null/undefined, a cadeia toda vale undefined.
    fn opcional(&mut self, opcional: bool) {
        if opcional {
            let r = *self.opcionais.last().expect("?. fora de cadeia opcional");
            self.salta(Op::SaltaSeNuloOuIndef, r);
        }
    }

    // ---------------------------------------------------------------- chamadas

    /// Avalia a função de uma chamada em `rf` e o `this` em `rf + 1`.
    fn funcao_e_this(&mut self, func: NoId) -> R<(u32, u32)> {
        let rf = self.temps(2);
        let rthis = rf + 1;
        match *self.arv.no(func) {
            No::Paren(x) if matches!(self.arv.no(x), No::Membro { .. } | No::Indice { .. }) => {
                self.liberar(rf);
                return self.funcao_e_this(x);
            }
            No::Membro { obj, prop, opcional } if !matches!(self.arv.no(obj), No::Super) => {
                self.expr(obj)?;
                self.opcional(opcional);
                self.op(Op::Guarda, &[Arg::U(rthis)]);
                let k = self.k_nome(prop);
                self.posicao(func);
                self.op(Op::LeProp, &[Arg::U(rthis), Arg::U(k), Arg::Cache]);
            }
            No::Indice { obj, prop, opcional } if !matches!(self.arv.no(obj), No::Super) => {
                self.expr(obj)?;
                self.opcional(opcional);
                self.op(Op::Guarda, &[Arg::U(rthis)]);
                self.expr(prop)?;
                self.posicao(func);
                self.op(Op::LeItem, &[Arg::U(rthis), Arg::Cache]);
            }
            No::MembroPrivado { obj, prop, opcional } => {
                self.expr(obj)?;
                self.opcional(opcional);
                self.op(Op::Guarda, &[Arg::U(rthis)]);
                let k = self.k_privado(prop);
                self.op(Op::LePrivado, &[Arg::U(rthis), Arg::U(k)]);
            }
            No::Membro { obj, .. } | No::Indice { obj, .. } if matches!(self.arv.no(obj), No::Super) => {
                self.membro(func)?;
                self.op(Op::Guarda, &[Arg::U(rf)]);
                self.op0(Op::CarregaThis);
                self.op(Op::Guarda, &[Arg::U(rthis)]);
                return Ok((rf, rthis));
            }
            _ => {
                self.expr(func)?;
                self.op(Op::Guarda, &[Arg::U(rf)]);
                self.op0(Op::CarregaUndef);
                self.op(Op::Guarda, &[Arg::U(rthis)]);
                return Ok((rf, rthis));
            }
        }
        self.op(Op::Guarda, &[Arg::U(rf)]);
        Ok((rf, rthis))
    }

    /// Avalia os argumentos em registradores seguidos. Com `...`, monta um array só.
    /// Devolve (primeiro registrador, quantos, se é espalhado).
    fn argumentos(&mut self, args: Lista) -> R<(u32, u32, bool)> {
        let lista = self.arv.lista(args).to_vec();
        if lista.iter().any(|&a| matches!(self.arv.no(a), No::Espalhar(_))) {
            let r = self.temp();
            self.op(Op::CriaArray, &[Arg::U(lista.len() as u32)]);
            self.op(Op::Guarda, &[Arg::U(r)]);
            for &a in &lista {
                if let No::Espalhar(e) = *self.arv.no(a) {
                    self.expr(e)?;
                    self.op(Op::EspalhaArray, &[Arg::U(r)]);
                } else {
                    self.expr(a)?;
                    self.op(Op::Empurra, &[Arg::U(r)]);
                }
            }
            return Ok((r, 1, true));
        }
        let r = self.temps(lista.len() as u32);
        for (i, &a) in lista.iter().enumerate() {
            self.expr(a)?;
            self.op(Op::Guarda, &[Arg::U(r + i as u32)]);
        }
        Ok((r, lista.len() as u32, false))
    }

    fn chamada(&mut self, n: NoId) -> R<()> {
        let No::Chamada { func, args, opcional } = *self.arv.no(n) else { unreachable!() };
        let base = self.f.topo;
        if matches!(self.arv.no(func), No::Super) {
            let (ra, na, espalha) = self.argumentos(args)?;
            self.posicao(n);
            self.op(if espalha { Op::SuperChamaEspalhado } else { Op::SuperChama }, &[Arg::U(ra), Arg::U(na)]);
            self.liberar(base);
            return Ok(());
        }
        let eval = matches!(self.arv.no(func), No::Nome(p::EVAL)) && !matches!(self.esc.referencia(func), Some(Ref::Vinculo(_)));
        let (rf, rthis) = self.funcao_e_this(func)?;
        if opcional {
            self.op(Op::Carrega, &[Arg::U(rf)]);
            self.opcional(true);
        }
        let (ra, na, espalha) = self.argumentos(args)?;
        debug_assert!(ra == rthis + 1 || espalha);
        self.posicao(n);
        if espalha {
            self.op(Op::ChamaEspalhado, &[Arg::U(rf), Arg::U(rthis), Arg::U(na)]);
        } else if eval {
            self.op(Op::ChamaEval, &[Arg::U(rf), Arg::U(rthis), Arg::U(na)]);
        } else {
            self.op(Op::Chama, &[Arg::U(rf), Arg::U(rthis), Arg::U(na), Arg::Cache]);
        }
        self.liberar(base);
        Ok(())
    }

    // ---------------------------------------------------------------- operadores

    fn unario(&mut self, n: NoId, op: OpUnario, arg: NoId) -> R<()> {
        match op {
            OpUnario::Typeof => {
                if let No::Nome(a) = *self.arv.no(arg) {
                    self.ler_nome(arg, a, true);
                } else {
                    self.expr(arg)?;
                }
                self.op0(Op::Typeof);
            }
            OpUnario::Delete => {
                let alvo = self.sem_parenteses(arg);
                match *self.arv.no(alvo) {
                    No::Membro { obj, prop, .. } => {
                        let r = self.expr_reg(obj)?;
                        let k = self.k_nome(prop);
                        self.posicao(n);
                        self.op(Op::ApagaProp, &[Arg::U(r), Arg::U(k)]);
                        self.liberar(r);
                    }
                    No::Indice { obj, prop, .. } => {
                        let r = self.expr_reg(obj)?;
                        self.expr(prop)?;
                        self.posicao(n);
                        self.op(Op::ApagaItem, &[Arg::U(r)]);
                        self.liberar(r);
                    }
                    // shortcut: `delete nome` (global implícito) dá true sem apagar; raro.
                    No::Nome(_) => self.op0(Op::CarregaVerdadeiro),
                    _ => {
                        self.expr(arg)?;
                        self.op0(Op::CarregaVerdadeiro);
                    }
                }
            }
            OpUnario::Void => {
                self.expr(arg)?;
                self.op0(Op::CarregaUndef);
            }
            OpUnario::Menos => {
                if let No::Numero(x) = *self.arv.no(arg) {
                    self.numero(-x);
                    return Ok(());
                }
                self.expr(arg)?;
                self.op(Op::Negativo, &[Arg::Cache]);
            }
            OpUnario::Mais => {
                self.expr(arg)?;
                self.op0(Op::ParaNumero);
            }
            OpUnario::Nao => {
                self.expr(arg)?;
                self.op0(Op::Nao);
            }
            OpUnario::Til => {
                self.expr(arg)?;
                self.op(Op::Til, &[Arg::Cache]);
            }
        }
        Ok(())
    }

    fn sem_parenteses(&self, mut n: NoId) -> NoId {
        while let No::Paren(x) = *self.arv.no(n) {
            n = x;
        }
        n
    }

    fn binario(&mut self, n: NoId, op: OpBinario, esq: NoId, dir: NoId) -> R<()> {
        if op == OpBinario::InPrivado {
            let No::Nome(a) = *self.arv.no(esq) else { return self.erro(esq, "#nome inválido") };
            self.expr(dir)?;
            let k = self.k_privado(a);
            self.op(Op::TemPrivado, &[Arg::U(k)]);
            return Ok(());
        }
        // `x + 1`, `i - 1`: soma com imediato, sem registrador.
        if let (OpBinario::Soma | OpBinario::Sub, No::Numero(v)) = (op, *self.arv.no(dir)) {
            if v.fract() == 0.0 && v.abs() < 1e9 && v != 0.0 {
                self.expr(esq)?;
                self.posicao(n);
                let i = if op == OpBinario::Soma { v as i32 } else { -(v as i32) };
                self.op(Op::SomaInt, &[Arg::I(i), Arg::Cache]);
                return Ok(());
            }
        }
        let r = self.expr_reg(esq)?;
        self.expr(dir)?;
        self.posicao(n);
        self.op_binario(op, r);
        self.liberar(r);
        Ok(())
    }

    /// acc = r op acc
    pub(crate) fn op_binario(&mut self, op: OpBinario, r: u32) {
        let (o, cache) = match op {
            OpBinario::Soma => (Op::Soma, true),
            OpBinario::Sub => (Op::Sub, true),
            OpBinario::Mul => (Op::Mul, true),
            OpBinario::Div => (Op::Div, true),
            OpBinario::Resto => (Op::Resto, true),
            OpBinario::Potencia => (Op::Potencia, true),
            OpBinario::DeslEsq => (Op::DeslEsq, true),
            OpBinario::DeslDir => (Op::DeslDir, true),
            OpBinario::DeslDirSemSinal => (Op::DeslDirSemSinal, true),
            OpBinario::E => (Op::E, true),
            OpBinario::Ou => (Op::Ou, true),
            OpBinario::OuExclusivo => (Op::OuExclusivo, true),
            OpBinario::Igual => (Op::Igual, true),
            OpBinario::Diferente => (Op::Diferente, true),
            OpBinario::Identico => (Op::Identico, false),
            OpBinario::NaoIdentico => (Op::NaoIdentico, false),
            OpBinario::Menor => (Op::Menor, true),
            OpBinario::Maior => (Op::Maior, true),
            OpBinario::MenorIgual => (Op::MenorIgual, true),
            OpBinario::MaiorIgual => (Op::MaiorIgual, true),
            OpBinario::In => (Op::In, false),
            OpBinario::Instanceof => (Op::Instanceof, true),
            OpBinario::InPrivado => unreachable!(),
        };
        if cache {
            self.op(o, &[Arg::U(r), Arg::Cache]);
        } else {
            self.op(o, &[Arg::U(r)]);
        }
    }

    // ---------------------------------------------------------------- atribuição

    /// Avalia as partes do alvo (objeto, chave) em registradores.
    pub(crate) fn lugar(&mut self, alvo: NoId) -> R<Lugar> {
        Ok(match *self.arv.no(alvo) {
            No::Nome(a) => Lugar::Nome(alvo, a),
            No::Paren(x) => return self.lugar(x),
            No::Membro { obj, prop, .. } => {
                let k = self.k_nome(prop);
                if matches!(self.arv.no(obj), No::Super) {
                    let t = self.temp();
                    self.op0(Op::CarregaThis);
                    self.op(Op::Guarda, &[Arg::U(t)]);
                    Lugar::Super(t, k)
                } else {
                    Lugar::Prop(self.expr_reg(obj)?, k)
                }
            }
            No::Indice { obj, prop, .. } => {
                if matches!(self.arv.no(obj), No::Super) {
                    let t = self.temp();
                    self.op0(Op::CarregaThis);
                    self.op(Op::Guarda, &[Arg::U(t)]);
                    let rk = self.expr_reg(prop)?;
                    Lugar::SuperItem(t, rk)
                } else {
                    let ro = self.expr_reg(obj)?;
                    let rk = self.temp();
                    self.expr(prop)?;
                    self.op0(Op::ParaChave);
                    self.op(Op::Guarda, &[Arg::U(rk)]);
                    Lugar::Item(ro, rk)
                }
            }
            No::MembroPrivado { obj, prop, .. } => {
                let k = self.k_privado(prop);
                Lugar::Privado(self.expr_reg(obj)?, k)
            }
            // Anexo B: `f() = 1` avalia `f()` e dá ReferenceError.
            No::Chamada { .. } => {
                self.expr(alvo)?;
                let k = self.k_texto("alvo de atribuição inválido".encode_utf16().collect());
                self.op(Op::LancaErro, &[Arg::U(k)]);
                Lugar::Prop(self.temp(), k)
            }
            _ => return self.erro(alvo, "alvo de atribuição inválido"),
        })
    }

    fn ler_lugar(&mut self, l: Lugar) {
        match l {
            Lugar::Nome(n, a) => self.ler_nome(n, a, false),
            Lugar::Prop(r, k) => self.op(Op::LeProp, &[Arg::U(r), Arg::U(k), Arg::Cache]),
            Lugar::Item(r, rk) => {
                self.op(Op::Carrega, &[Arg::U(rk)]);
                self.op(Op::LeItem, &[Arg::U(r), Arg::Cache]);
            }
            Lugar::Privado(r, k) => self.op(Op::LePrivado, &[Arg::U(r), Arg::U(k)]),
            Lugar::Super(t, k) => self.op(Op::LeSuper, &[Arg::U(t), Arg::U(k)]),
            Lugar::SuperItem(t, rk) => self.op(Op::LeSuperItem, &[Arg::U(t), Arg::U(rk)]),
        }
    }

    pub(crate) fn gravar_lugar(&mut self, l: Lugar) {
        match l {
            Lugar::Nome(n, a) => self.gravar_nome(n, a, false),
            Lugar::Prop(r, k) => self.op(Op::GravaProp, &[Arg::U(r), Arg::U(k), Arg::Cache]),
            Lugar::Item(r, rk) => self.op(Op::GravaItem, &[Arg::U(r), Arg::U(rk), Arg::Cache]),
            Lugar::Privado(r, k) => self.op(Op::GravaPrivado, &[Arg::U(r), Arg::U(k)]),
            Lugar::Super(t, k) => self.op(Op::GravaSuper, &[Arg::U(t), Arg::U(k)]),
            Lugar::SuperItem(t, rk) => {
                let v = self.temp();
                self.op(Op::Guarda, &[Arg::U(v)]);
                self.op(Op::GravaSuperItem, &[Arg::U(t), Arg::U(rk), Arg::U(v)]);
                self.op(Op::Carrega, &[Arg::U(v)]);
            }
        }
    }

    fn atribuicao(&mut self, n: NoId, op: OpAtrib, alvo: NoId, valor: NoId) -> R<()> {
        let base = self.f.topo;
        self.posicao(n);
        if let No::PadraoArray(_) | No::PadraoObjeto(_) = self.arv.no(alvo) {
            let t = self.temp();
            self.expr(valor)?;
            self.op(Op::Guarda, &[Arg::U(t)]);
            self.atribuir_padrao(alvo, false)?;
            self.op(Op::Carrega, &[Arg::U(t)]);
            self.liberar(base);
            return Ok(());
        }
        let lugar = self.lugar(alvo)?;
        match op {
            OpAtrib::Simples => {
                if let Lugar::Nome(_, a) = lugar {
                    let u: Vec<u16> = self.lex.atomos.texto(a).encode_utf16().collect();
                    self.expr_nomeada(valor, u)?;
                } else {
                    self.expr(valor)?;
                }
                self.posicao(n);
                self.gravar_lugar(lugar);
            }
            OpAtrib::Bin(b) => {
                self.ler_lugar(lugar);
                let velho = self.temp();
                self.op(Op::Guarda, &[Arg::U(velho)]);
                self.expr(valor)?;
                self.posicao(n);
                self.op_binario(b, velho);
                self.gravar_lugar(lugar);
            }
            OpAtrib::Log(l) => {
                let fim = self.rotulo();
                self.ler_lugar(lugar);
                self.salto_logico(l, fim);
                if let Lugar::Nome(_, a) = lugar {
                    let u: Vec<u16> = self.lex.atomos.texto(a).encode_utf16().collect();
                    self.expr_nomeada(valor, u)?;
                } else {
                    self.expr(valor)?;
                }
                self.gravar_lugar(lugar);
                self.por(fim);
            }
        }
        self.liberar(base);
        Ok(())
    }
}

/// `"0"`..`"4294967294"` sem zero à esquerda (§6.1.7), como `indice_de_array` do runtime.
fn e_indice(u: &[u16]) -> bool {
    !u.is_empty() && u.len() <= 10 && (u.len() == 1 || u[0] != u16::from(b'0')) && u.iter().all(|c| (u16::from(b'0')..=u16::from(b'9')).contains(c))
        && String::from_utf16_lossy(u).parse::<u64>().is_ok_and(|n| n < u64::from(u32::MAX))
}
