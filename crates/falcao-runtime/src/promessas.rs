//! Promessas (§27.2) e a fila de microtarefas.
//!
//! Campos internos da promessa: `[estado cru][resultado][reações (lista)][tratada cru]`.
//! Reações em trincas `[ao_cumprir, ao_rejeitar, derivada]`; continuação de async é
//! `[int(id), BURACO, undefined]`.

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::interpretador::Entrada;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

pub(crate) const PENDENTE: u32 = 0;
pub(crate) const CUMPRIDA: u32 = 1;
pub(crate) const REJEITADA: u32 = 2;

/// Trabalho da fila de microtarefas.
#[derive(Clone, Copy, Debug)]
pub enum Tarefa {
    /// Roda o tratador (`undefined` = repassa) com `arg` e resolve a derivada.
    Reacao { f: Valor, arg: Valor, derivada: Valor, rejeitou: bool },
    /// Continua a async `id`.
    Async { id: u32, valor: Valor, erro: bool },
    /// `then` de um thenable que resolveu a promessa `p`.
    Thenable { p: Valor, x: Valor, then: Valor },
    /// `queueMicrotask(f)`.
    Funcao { f: Valor },
}

impl Tarefa {
    pub fn valores_mut(&mut self) -> Vec<&mut Valor> {
        match self {
            Tarefa::Reacao { f, arg, derivada, .. } => vec![f, arg, derivada],
            Tarefa::Async { valor, .. } => vec![valor],
            Tarefa::Thenable { p, x, then } => vec![p, x, then],
            Tarefa::Funcao { f } => vec![f],
        }
    }
}

impl Motor {
    pub fn e_promessa(&self, v: Valor) -> bool {
        self.e_objeto(v) && self.formas.get(self.forma(v)).classe == Classe::Promessa
    }

    pub fn nova_promessa(&mut self) -> Valor {
        let p = self.intr.promessa_proto;
        self.promessa_com_proto(p)
    }

    fn promessa_com_proto(&mut self, proto: Valor) -> Valor {
        let f = self.forma_para(Classe::Promessa, proto, 0);
        let p = self.novo_objeto(f);
        self.por_interno(p, 0, PENDENTE);
        self.por_interno_valor(p, 2, Valor::LISTA_VAZIA);
        p
    }

    /// `Promise.resolve(v)`: a própria se já é promessa nossa.
    pub fn promessa_de(&mut self, v: Valor) -> Valor {
        if self.e_promessa(v) && self.proto_de(v) == self.intr.promessa_proto {
            return v;
        }
        let p = self.nova_promessa();
        self.resolver_promessa(p, v);
        p
    }

    /// Procedimento de resolução (thenables viram microtarefa).
    pub fn resolver_promessa(&mut self, p: Valor, x: Valor) {
        if !self.e_promessa(p) || self.interno(p, 0) != PENDENTE {
            return;
        }
        if x == p {
            let e = self.erro(crate::intrinsecos::TipoErro::Tipo, "Chaining cycle detected for promise");
            return self.rejeitar_promessa(p, e);
        }
        if self.e_objeto(x) {
            let then = match self.obter(x, ChaveProp::Nome(fixos::THEN)) {
                Ok(t) => t,
                Err(e) => return self.rejeitar_promessa(p, e),
            };
            if self.e_funcao(then) {
                self.microtarefas.push_back(Tarefa::Thenable { p, x, then });
                return;
            }
        }
        self.assentar(p, CUMPRIDA, x);
    }

    pub fn rejeitar_promessa(&mut self, p: Valor, e: Valor) {
        if self.e_promessa(p) && self.interno(p, 0) == PENDENTE {
            self.assentar(p, REJEITADA, e);
        }
    }

    fn assentar(&mut self, p: Valor, estado: u32, v: Valor) {
        let reacoes = Valor(self.interno(p, 2));
        self.por_interno(p, 0, estado);
        self.por_interno_valor(p, 1, v);
        self.por_interno_valor(p, 2, Valor::LISTA_VAZIA);
        // Rejeitada sem ninguém escutando: fica anotada até o fim da volta do laço.
        if estado == REJEITADA && self.interno(p, 3) == 0 {
            self.rejeicoes.push(p);
        }
        if reacoes == Valor::LISTA_VAZIA {
            return;
        }
        let n = self.lista_len(reacoes);
        let mut i = 0;
        while i + 3 <= n {
            let (ok, err, d) = (self.lista_get(reacoes, i), self.lista_get(reacoes, i + 1), self.lista_get(reacoes, i + 2));
            self.enfileirar_reacao(ok, err, d, estado, v);
            i += 3;
        }
    }

    fn enfileirar_reacao(&mut self, ok: Valor, err: Valor, derivada: Valor, estado: u32, v: Valor) {
        if err == Valor::BURACO {
            self.microtarefas.push_back(Tarefa::Async { id: ok.como_int() as u32, valor: v, erro: estado == REJEITADA });
            return;
        }
        let rejeitou = estado == REJEITADA;
        let f = if rejeitou { err } else { ok };
        self.microtarefas.push_back(Tarefa::Reacao { f, arg: v, derivada, rejeitou });
    }

    fn adicionar_reacao(&mut self, p: Valor, ok: Valor, err: Valor, derivada: Valor) {
        let tratada = self.interno(p, 3) != 0;
        self.por_interno(p, 3, 1);
        let estado = self.interno(p, 0);
        if !tratada && estado == REJEITADA {
            self.rejeicoes.retain(|&x| x != p);
        }
        if estado != PENDENTE {
            let v = Valor(self.interno(p, 1));
            return self.enfileirar_reacao(ok, err, derivada, estado, v);
        }
        let l = Valor(self.interno(p, 2));
        let n = if l == Valor::LISTA_VAZIA { 0 } else { self.lista_len(l) };
        let nova = self.redimensionar_lista(l, n + 3, Valor::UNDEFINED);
        self.lista_set(nova, n, ok);
        self.lista_set(nova, n + 1, err);
        self.lista_set(nova, n + 2, derivada);
        self.por_interno_valor(p, 2, nova);
    }

    /// Estado de uma promessa: `None` = pendente; `Some((rejeitada, resultado))`.
    pub fn estado_promessa(&self, p: Valor) -> Option<(bool, Valor)> {
        match self.interno(p, 0) {
            PENDENTE => None,
            e => Some((e == REJEITADA, Valor(self.interno(p, 1)))),
        }
    }

    /// Conta a promessa como tratada (quem chamou cuida do erro dela).
    pub fn marcar_tratada(&mut self, p: Valor) {
        if self.e_promessa(p) && self.interno(p, 3) == 0 {
            self.por_interno(p, 3, 1);
            self.rejeicoes.retain(|&x| x != p);
        }
    }

    /// Os motivos das promessas rejeitadas que continuam sem tratador (e esvazia a lista).
    /// Chamar depois de [`Motor::rodar_microtarefas`]: é o "Uncaught (in promise)".
    pub fn tirar_rejeicoes_nao_tratadas(&mut self) -> Vec<Valor> {
        let l = std::mem::take(&mut self.rejeicoes);
        l.into_iter().filter(|&p| self.interno(p, 3) == 0).map(|p| Valor(self.interno(p, 1))).collect()
    }

    /// A async `id` continua quando `p` assentar.
    pub(crate) fn reagir_async(&mut self, p: Valor, id: u32) {
        self.adicionar_reacao(p, Valor::int(id as i32), Valor::BURACO, Valor::UNDEFINED);
    }

    /// `p.then(ok, err)` sem passar pelo JS.
    pub fn entao(&mut self, p: Valor, ok: Valor, err: Valor) -> Valor {
        let d = self.nova_promessa();
        let ok = if self.e_funcao(ok) { ok } else { Valor::UNDEFINED };
        let err = if self.e_funcao(err) { err } else { Valor::UNDEFINED };
        self.adicionar_reacao(p, ok, err, d);
        d
    }

    /// Par de funções `resolve`/`reject` para a promessa.
    pub fn funcoes_resolucao(&mut self, p: Valor) -> (Valor, Valor) {
        let (r, j) = (self.intr.nativa_resolver, self.intr.nativa_rejeitar);
        (self.nativa_com(r, 1, p), self.nativa_com(j, 1, p))
    }

    /// Roda as microtarefas até a fila esvaziar.
    /// No topo (`aninhamento == 0`) o coletor pode rodar dentro das chamadas: o que a tarefa
    /// usa depois da chamada passa por `raizes`.
    pub fn rodar_microtarefas(&mut self) {
        while let Some(t) = self.microtarefas.pop_front() {
            match t {
                Tarefa::Reacao { f, arg, mut derivada, rejeitou } => {
                    let r = if f == Valor::UNDEFINED {
                        if rejeitou {
                            Err(arg)
                        } else {
                            Ok(arg)
                        }
                    } else {
                        self.raizes.push(derivada);
                        let r = self.chamar_externo(f, Valor::UNDEFINED, &[arg]);
                        derivada = self.raizes.pop().unwrap();
                        r
                    };
                    if derivada != Valor::UNDEFINED {
                        match r {
                            Ok(v) => self.resolver_promessa(derivada, v),
                            Err(e) => self.rejeitar_promessa(derivada, e),
                        }
                    }
                }
                Tarefa::Async { id, valor, erro } => {
                    let ent = if erro { Entrada::Lanca(valor) } else { Entrada::Normal(valor) };
                    let r = if self.aninhamento == 0 { self.retomar_no_topo(id, ent) } else { self.retomar(id, ent) };
                    self.depois_async(id, r);
                }
                Tarefa::Thenable { p, x, then } => {
                    let (res, rej) = self.funcoes_resolucao(p);
                    self.raizes.push(p);
                    let r = self.chamar_externo(then, x, &[res, rej]);
                    let p = self.raizes.pop().unwrap();
                    if let Err(e) = r {
                        self.rejeitar_promessa(p, e);
                    }
                }
                Tarefa::Funcao { f } => {
                    if let Err(e) = self.chamar_externo(f, Valor::UNDEFINED, &[]) {
                        self.relatar_erro(e);
                    }
                }
            }
        }
    }

    /// Mostra um erro não tratado no console.
    pub fn relatar_erro(&mut self, e: Valor) {
        let s = self.texto_do_erro(e);
        self.imprimir(2, &format!("Uncaught {s}"));
    }

    /// `Nome: mensagem` + pilha, para mostrar.
    pub fn texto_do_erro(&mut self, e: Valor) -> String {
        if self.e_objeto(e) {
            let stack = self.obter(e, ChaveProp::Nome(fixos::STACK)).unwrap_or(Valor::UNDEFINED);
            let base = match self.para_texto(e) {
                Ok(t) => self.texto_rust(t),
                Err(_) => "erro".into(),
            };
            if self.e_texto(stack) {
                let s = self.texto_rust(stack);
                return if s.starts_with(&base) { s } else { format!("{base}{s}") };
            }
            return base;
        }
        self.para_texto_rust(e).unwrap_or_else(|_| "erro".into())
    }

    pub fn imprimir(&mut self, nivel: u8, s: &str) {
        match self.ao_imprimir.as_mut() {
            Some(f) => f(nivel, s),
            None => println!("{s}"),
        }
    }

    // ---------------------------------------------------------------- builtins

    pub(crate) fn instalar_promessas(&mut self) {
        let op = self.intr.objeto_proto;
        let pp = self.forma_para(Classe::Comum, op, 0);
        let proto = self.novo_objeto(pp);
        self.intr.promessa_proto = proto;
        self.intr.nativa_resolver = self.registrar_nativa("", resolver_nativa, false);
        self.intr.nativa_rejeitar = self.registrar_nativa("", rejeitar_nativa, false);
        self.intr.nativa_todos = self.registrar_nativa("", todos_elemento, false);
        let ctor = self.nativa("Promise", 1, construtor, true);
        self.definir_propria(ctor, fixos::PROTOTYPE, proto, 0);
        self.definir_propria(proto, fixos::CONSTRUCTOR, ctor, attr::METODO);
        let tag = self.texto("Promise");
        self.definir_propria(proto, crate::intrinsecos::simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
        self.metodo(proto, "then", 2, then);
        self.metodo(proto, "catch", 1, catch);
        self.metodo(proto, "finally", 1, finally);
        self.metodo(ctor, "resolve", 1, resolve);
        self.metodo(ctor, "reject", 1, reject);
        self.metodo(ctor, "all", 1, all);
        self.metodo(ctor, "allSettled", 1, all_settled);
        self.metodo(ctor, "race", 1, race);
        self.metodo(ctor, "any", 1, any);
        self.metodo(ctor, "withResolvers", 0, with_resolvers);
        let g = self.intr.global;
        let c = self.chave("Promise");
        self.definir_propria(g, c, ctor, attr::METODO);
        self.metodo(g, "queueMicrotask", 1, queue_microtask);
    }
}

fn resolver_nativa(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    let x = m.arg(a, 0);
    m.resolver_promessa(p, x);
    Ok(Valor::UNDEFINED)
}

fn rejeitar_nativa(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.capturado();
    let x = m.arg(a, 0);
    m.rejeitar_promessa(p, x);
    Ok(Valor::UNDEFINED)
}

fn construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Promise constructor cannot be invoked without 'new'");
    }
    let exec = m.arg(a, 0);
    if !m.e_funcao(exec) {
        return m.erro_tipo("Promise resolver is not a function");
    }
    let pp = m.intr.promessa_proto;
    let proto = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let proto = if m.e_objeto(proto) { proto } else { pp };
    let p = m.promessa_com_proto(proto);
    let (res, rej) = m.funcoes_resolucao(p);
    if let Err(e) = m.chamar(exec, Valor::UNDEFINED, &[res, rej]) {
        m.rejeitar_promessa(p, e);
    }
    Ok(p)
}

fn exigir_promessa(m: &mut Motor, v: Valor) -> Res<()> {
    if m.e_promessa(v) {
        Ok(())
    } else {
        m.erro_tipo("Method Promise.prototype.then called on incompatible receiver")
    }
}

fn then(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    exigir_promessa(m, this)?;
    let (ok, err) = (m.arg(a, 0), m.arg(a, 1));
    Ok(m.entao(this, ok, err))
}

fn catch(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    let t = m.obter(this, ChaveProp::Nome(fixos::THEN))?;
    m.chamar(t, this, &[Valor::UNDEFINED, f])
}

fn finally_ok(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.capturado();
    let r = m.chamar(f, Valor::UNDEFINED, &[])?;
    let v = m.arg(a, 0);
    // shortcut: não espera a promessa que o tratador do finally devolve.
    let _ = r;
    Ok(v)
}

fn finally_err(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.capturado();
    m.chamar(f, Valor::UNDEFINED, &[])?;
    Err(m.arg(a, 0))
}

fn finally(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    let t = m.obter(this, ChaveProp::Nome(fixos::THEN))?;
    if !m.e_funcao(f) {
        return m.chamar(t, this, &[f, f]);
    }
    let i1 = m.id_nativa("finally_ok", finally_ok);
    let i2 = m.id_nativa("finally_err", finally_err);
    let ok = m.nativa_com(i1, 1, f);
    let err = m.nativa_com(i2, 1, f);
    m.chamar(t, this, &[ok, err])
}

fn resolve(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    Ok(m.promessa_de(v))
}

fn reject(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.nova_promessa();
    let e = m.arg(a, 0);
    m.rejeitar_promessa(p, e);
    Ok(p)
}

fn with_resolvers(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let p = m.nova_promessa();
    let (res, rej) = m.funcoes_resolucao(p);
    let o = m.objeto_vazio();
    let kp = m.chave("promise");
    let kr = m.chave("resolve");
    let kj = m.chave("reject");
    m.definir_propria(o, kp, p, attr::PADRAO);
    m.definir_propria(o, kr, res, attr::PADRAO);
    m.definir_propria(o, kj, rej, attr::PADRAO);
    Ok(o)
}

// Estado comum de all/allSettled/any: lista [resultados (array), restantes (int), derivada, modo].
const MODO_ALL: i32 = 0;
const MODO_SETTLED_OK: i32 = 1;
const MODO_SETTLED_ERR: i32 = 2;
const MODO_ANY: i32 = 3;

/// Tratador de um item: captura `[estado, índice, modo]`.
fn todos_elemento(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let cap = m.capturado();
    let (estado, i, modo) = (m.lista_get(cap, 0), m.lista_get(cap, 1).como_int() as u32, m.lista_get(cap, 2).como_int());
    if m.lista_get(cap, 3) == Valor::TRUE {
        return Ok(Valor::UNDEFINED);
    }
    m.lista_set(cap, 3, Valor::TRUE);
    let v = m.arg(a, 0);
    let item = match modo {
        MODO_SETTLED_OK | MODO_SETTLED_ERR => {
            let o = m.objeto_vazio();
            let ks = m.chave("status");
            let (st, kv) = if modo == MODO_SETTLED_OK { ("fulfilled", fixos::VALUE) } else { ("rejected", m.chave("reason")) };
            let st = m.texto(st);
            m.definir_propria(o, ks, st, attr::PADRAO);
            m.definir_propria(o, kv, v, attr::PADRAO);
            o
        }
        _ => v,
    };
    let res = m.lista_get(estado, 0);
    m.gravar_elemento(res, i, item);
    let n = m.lista_get(estado, 1).como_int() - 1;
    m.lista_set(estado, 1, Valor::int(n));
    if n == 0 {
        let d = m.lista_get(estado, 2);
        if modo == MODO_ANY {
            let e = m.erro(crate::intrinsecos::TipoErro::Agregado, "All promises were rejected");
            m.definir_propria(e, fixos::ERRORS, res, attr::METODO);
            m.rejeitar_promessa(d, e);
        } else {
            m.resolver_promessa(d, res);
        }
    }
    Ok(Valor::UNDEFINED)
}

fn combinar(m: &mut Motor, a: Args, modo: i32) -> Res<Valor> {
    let d = m.nova_promessa();
    let itens = match m.coletar_iteravel(m.arg(a, 0)) {
        Ok(i) => i,
        Err(e) => {
            m.rejeitar_promessa(d, e);
            return Ok(d);
        }
    };
    let res = m.array_de(&[]);
    if !itens.is_empty() {
        m.definir_array_len(res, itens.len() as u32);
    }
    let estado = m.nova_lista(4);
    m.lista_set(estado, 0, res);
    m.lista_set(estado, 1, Valor::int(itens.len() as i32));
    m.lista_set(estado, 2, d);
    if itens.is_empty() {
        if modo == MODO_ANY {
            let e = m.erro(crate::intrinsecos::TipoErro::Agregado, "All promises were rejected");
            m.rejeitar_promessa(d, e);
        } else {
            m.resolver_promessa(d, res);
        }
        return Ok(d);
    }
    let id = m.intr.nativa_todos;
    let (rr, rj) = m.funcoes_resolucao(d);
    for (i, x) in itens.into_iter().enumerate() {
        let p = m.promessa_de(x);
        let cap = |m: &mut Motor, modo: i32| {
            let c = m.nova_lista(4);
            m.lista_set(c, 0, estado);
            m.lista_set(c, 1, Valor::int(i as i32));
            m.lista_set(c, 2, Valor::int(modo));
            m.lista_set(c, 3, Valor::FALSE);
            m.nativa_com(id, 1, c)
        };
        let (ok, err) = match modo {
            MODO_ALL => (cap(m, MODO_ALL), rj),
            MODO_ANY => (rr, cap(m, MODO_ANY)),
            _ => (cap(m, MODO_SETTLED_OK), cap(m, MODO_SETTLED_ERR)),
        };
        m.entao(p, ok, err);
    }
    Ok(d)
}

fn all(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    combinar(m, a, MODO_ALL)
}

fn all_settled(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    combinar(m, a, MODO_SETTLED_OK)
}

fn any(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    combinar(m, a, MODO_ANY)
}

fn race(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let d = m.nova_promessa();
    let itens = match m.coletar_iteravel(m.arg(a, 0)) {
        Ok(i) => i,
        Err(e) => {
            m.rejeitar_promessa(d, e);
            return Ok(d);
        }
    };
    let (rr, rj) = m.funcoes_resolucao(d);
    for x in itens {
        let p = m.promessa_de(x);
        m.entao(p, rr, rj);
    }
    Ok(d)
}

fn queue_microtask(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return m.erro_tipo("queueMicrotask: o argumento não é uma função");
    }
    m.microtarefas.push_back(Tarefa::Funcao { f });
    Ok(Valor::UNDEFINED)
}
