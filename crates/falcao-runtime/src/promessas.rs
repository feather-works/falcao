//! Promessas (§27.2) e a fila de microtarefas.
//!
//! Campos internos da promessa: `[estado cru][resultado][reações (lista)][tratada cru]`.
//! Reações em trincas `[ao_cumprir, ao_rejeitar, derivada]`; continuação de async é
//! `[int(id), BURACO, undefined]`. A derivada é uma promessa nossa ou, quando `then` caiu num
//! construtor de espécie de fora, a capacidade `[promessa, resolve, reject]` (array do JS).

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
        let c = self.nova_lista(2);
        self.lista_set(c, 0, p);
        self.lista_set(c, 1, Valor::FALSE);
        (self.nativa_com(r, 1, c), self.nativa_com(j, 1, c))
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
                    if self.e_promessa(derivada) {
                        match r {
                            Ok(v) => self.resolver_promessa(derivada, v),
                            Err(e) => self.rejeitar_promessa(derivada, e),
                        }
                    } else if derivada != Valor::UNDEFINED {
                        let (i, v) = match r {
                            Ok(v) => (1, v),
                            Err(e) => (2, e),
                        };
                        if let Ok(f) = self.obter_indice(derivada, i) {
                            if let Err(e) = self.chamar_externo(f, Valor::UNDEFINED, &[v]) {
                                self.relatar_erro(e);
                            }
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
                    self.raizes.push(rej);
                    let r = self.chamar_externo(then, x, &[res, rej]);
                    let rej = self.raizes.pop().unwrap();
                    if let Err(e) = r {
                        let _ = self.chamar_externo(rej, Valor::UNDEFINED, &[e]);
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
        let ctor = self.nativa("Promise", 1, construtor, true);
        self.intr.promessa_ctor = ctor;
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
        self.metodo(ctor, "try", 1, try_);
        self.metodo(ctor, "allKeyed", 1, all_keyed);
        self.metodo(ctor, "allSettledKeyed", 1, all_settled_keyed);
        let g = self.intr.global;
        let c = self.chave("Promise");
        self.definir_propria(g, c, ctor, attr::METODO);
        self.metodo(g, "queueMicrotask", 1, queue_microtask);
    }
}

/// A promessa da célula `[promessa, já resolvida]` do par, ou None se o par já foi usado.
fn tomar_celula(m: &mut Motor) -> Option<Valor> {
    let c = m.capturado();
    if m.lista_get(c, 1) == Valor::TRUE {
        return None;
    }
    m.lista_set(c, 1, Valor::TRUE);
    Some(m.lista_get(c, 0))
}

fn resolver_nativa(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if let Some(p) = tomar_celula(m) {
        let x = m.arg(a, 0);
        m.resolver_promessa(p, x);
    }
    Ok(Valor::UNDEFINED)
}

fn rejeitar_nativa(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if let Some(p) = tomar_celula(m) {
        let x = m.arg(a, 0);
        m.rejeitar_promessa(p, x);
    }
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
        m.chamar(rej, Valor::UNDEFINED, &[e])?;
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

/// SpeciesConstructor(o, %Promise%).
fn especie(m: &mut Motor, o: Valor) -> Res<Valor> {
    let padrao = m.intr.promessa_ctor;
    let c = m.obter(o, ChaveProp::Nome(fixos::CONSTRUCTOR))?;
    if c == Valor::UNDEFINED {
        return Ok(padrao);
    }
    if !m.e_objeto(c) {
        return m.erro_tipo("object.constructor is not an object");
    }
    let sp = m.obter(c, ChaveProp::Nome(crate::intrinsecos::simb::SPECIES))?;
    if sp == Valor::UNDEFINED || sp == Valor::NULL {
        return Ok(padrao);
    }
    if m.e_construtor(sp) {
        return Ok(sp);
    }
    m.erro_tipo("object.constructor[Symbol.species] is not a constructor")
}

fn then(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    exigir_promessa(m, this)?;
    let c = especie(m, this)?;
    let (ok, err) = (m.arg(a, 0), m.arg(a, 1));
    if c == m.intr.promessa_ctor {
        return Ok(m.entao(this, ok, err));
    }
    let cap = chamar_js(m, JS_CAPACIDADE, &[c])?;
    let ok = if m.e_funcao(ok) { ok } else { Valor::UNDEFINED };
    let err = if m.e_funcao(err) { err } else { Valor::UNDEFINED };
    m.adicionar_reacao(this, ok, err, cap);
    m.obter_indice(cap, 0)
}

fn catch(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    let t = m.obter(this, ChaveProp::Nome(fixos::THEN))?;
    m.chamar(t, this, &[Valor::UNDEFINED, f])
}

// ---------------------------------------------------------------- partes em JS

// Índices na lista que PROMESSA_JS devolve.
const JS_CAPACIDADE: u32 = 0;
const JS_ALL: u32 = 1;
const JS_ALL_SETTLED: u32 = 2;
const JS_ANY: u32 = 3;
const JS_RACE: u32 = 4;
const JS_ALL_KEYED: u32 = 5;
const JS_ALL_SETTLED_KEYED: u32 = 6;
const JS_TRY: u32 = 7;
const JS_RESOLVE: u32 = 8;
const JS_REJECT: u32 = 9;
const JS_WITH_RESOLVERS: u32 = 10;
const JS_FINALLY: u32 = 11;

/// Os combinadores, `finally` e os estáticos que aceitam qualquer construtor em `this`
/// (NewPromiseCapability). Só ajudantes nativos capturados: mexer no global não muda nada.
/// As funções que o usuário vê (elementos, executor, thenFinally) passam por `anon` para
/// nascerem sem nome, como pede a especificação.
const PROMESSA_JS: &str = r#"(function (eConstrutor, eObjeto, ePromessa, definir, chamar, erroTipo, erroAgregado, especie, chaves, enumeravel, objetoNulo, ITERADOR) {
'use strict';
const anon = f => f;
const FIM = [];
function capacidade(C) {
  if (!eConstrutor(C)) throw erroTipo('Promise capability: not a constructor');
  let res, rej;
  const p = new C(anon((a, b) => {
    if (res !== undefined) throw erroTipo('Promise executor has already been invoked with non-undefined arguments');
    if (rej !== undefined) throw erroTipo('Promise executor has already been invoked with non-undefined arguments');
    res = a;
    rej = b;
  }));
  if (typeof res !== 'function') throw erroTipo('Promise resolve function is not callable');
  if (typeof rej !== 'function') throw erroTipo('Promise reject function is not callable');
  return [p, res, rej];
}
function resolverCom(C, x) {
  if (ePromessa(x) && x.constructor === C) return x;
  const cap = capacidade(C);
  chamar(cap[1], undefined, [x]);
  return cap[0];
}
function rejeitar(cap, e) {
  chamar(cap[2], undefined, [e]);
  return cap[0];
}
function obterResolve(C) {
  const r = C.resolve;
  if (typeof r !== 'function') throw erroTipo('Promise resolve is not a function');
  return r;
}
function copia(l) {
  const a = [];
  for (let k = 0; k < l.length; k++) definir(a, k, l[k]);
  return a;
}
// reg = [iterador, next, feito]; qualquer erro ao andar deixa feito = true (não fecha).
function passo(reg) {
  reg[2] = true;
  const r = chamar(reg[1], reg[0], []);
  if (!eObjeto(r)) throw erroTipo('Iterator result is not an object');
  if (r.done) return FIM;
  const v = r.value;
  reg[2] = false;
  return v;
}
function fechar(reg) {
  try { const f = reg[0].return; if (f !== undefined && f !== null) chamar(f, reg[0], []); } catch (_) {}
}
function combinador(C, iteravel, laco) {
  const cap = capacidade(C);
  let pr, reg;
  try {
    pr = obterResolve(C);
    const f = iteravel[ITERADOR];
    if (typeof f !== 'function') throw erroTipo('object is not iterable');
    const it = chamar(f, iteravel, []);
    if (!eObjeto(it)) throw erroTipo('Result of the Symbol.iterator method is not an object');
    reg = [it, it.next, false];
  } catch (e) {
    return rejeitar(cap, e);
  }
  try {
    laco(C, cap, pr, reg);
  } catch (e) {
    if (!reg[2]) fechar(reg);
    return rejeitar(cap, e);
  }
  return cap[0];
}
// modo: 0 all, 1 allSettled, 2 any. `proximo()` devolve [chave, valor] ou FIM.
function juntar(C, cap, pr, modo, proximo, acabou) {
  const valores = [];
  const nomes = [];
  let restante = 1;
  const pronto = () => {
    if (modo === 2) chamar(cap[2], undefined, [erroAgregado(copia(valores))]);
    else chamar(cap[1], undefined, [acabou(nomes, valores)]);
  };
  for (let i = 0; ; i++) {
    const kv = proximo();
    if (kv === FIM) break;
    definir(nomes, i, kv[0]);
    definir(valores, i, undefined);
    const prox = chamar(pr, C, [kv[1]]);
    let chamado = false;
    const elemento = (tipo) => anon((x) => {
      if (chamado) return undefined;
      chamado = true;
      definir(valores, i, tipo === 0 ? x : tipo === 1 ? { status: 'fulfilled', value: x } : { status: 'rejected', reason: x });
      if (--restante === 0) pronto();
    });
    restante++;
    if (modo === 0) prox.then(elemento(0), cap[2]);
    else if (modo === 1) prox.then(elemento(1), elemento(2));
    else prox.then(cap[1], elemento(0));
  }
  if (--restante === 0) {
    if (modo === 2) throw erroAgregado(copia(valores));
    chamar(cap[1], undefined, [acabou(nomes, valores)]);
  }
}
const comoArray = (nomes, valores) => copia(valores);
function comoObjeto(nomes, valores) {
  const o = objetoNulo();
  for (let k = 0; k < nomes.length; k++) definir(o, nomes[k], valores[k]);
  return o;
}
const deIterador = (reg) => () => { const v = passo(reg); return v === FIM ? FIM : [undefined, v]; };
function todos(modo) {
  return (C, iteravel) => combinador(C, iteravel, (C, cap, pr, reg) => juntar(C, cap, pr, modo, deIterador(reg), comoArray));
}
function chaveado(modo) {
  return (C, o) => {
    const cap = capacidade(C);
    try {
      const pr = obterResolve(C);
      if (!eObjeto(o)) throw erroTipo('Promise.allKeyed: argument is not an object');
      const ks = chaves(o);
      let j = 0;
      const proximo = () => {
        while (j < ks.length) {
          const k = ks[j++];
          if (enumeravel(o, k)) return [k, o[k]];
        }
        return FIM;
      };
      juntar(C, cap, pr, modo, proximo, comoObjeto);
    } catch (e) {
      return rejeitar(cap, e);
    }
    return cap[0];
  };
}
function race(C, iteravel) {
  return combinador(C, iteravel, (C, cap, pr, reg) => {
    for (;;) {
      const v = passo(reg);
      if (v === FIM) return;
      chamar(pr, C, [v]).then(cap[1], cap[2]);
    }
  });
}
// Valor normal passa por PromiseResolve: uma promessa de C volta sem embrulho.
function tentar(C, f, args) {
  if (!eObjeto(C)) throw erroTipo('Promise.try called on non-object');
  let s;
  try { s = chamar(f, undefined, args); } catch (e) { return rejeitar(capacidade(C), e); }
  return resolverCom(C, s);
}
function resolve(C, x) {
  if (!eObjeto(C)) throw erroTipo('Promise.resolve called on non-object');
  return resolverCom(C, x);
}
function reject(C, r) {
  return rejeitar(capacidade(C), r);
}
function withResolvers(C) {
  const cap = capacidade(C);
  return { promise: cap[0], resolve: cap[1], reject: cap[2] };
}
function aoFim(p, f) {
  if (!eObjeto(p)) throw erroTipo('Promise.prototype.finally called on non-object');
  const C = especie(p);
  let ok = f, err = f;
  if (typeof f === 'function') {
    ok = anon((valor) => resolverCom(C, chamar(f, undefined, [])).then(anon(() => valor)));
    err = anon((motivo) => resolverCom(C, chamar(f, undefined, [])).then(anon(() => { throw motivo; })));
  }
  return p.then(ok, err);
}
return [capacidade, todos(0), todos(1), todos(2), race, chaveado(0), chaveado(1), tentar, resolve, reject, withResolvers, aoFim];
})"#;

/// Chama a parte `i` de PROMESSA_JS (compila no primeiro uso).
fn chamar_js(m: &mut Motor, i: u32, args: &[Valor]) -> Res<Valor> {
    if m.intr.promessa_js == Valor::UNDEFINED {
        let fabrica = {
            let p = m.compilar_texto(PROMESSA_JS)?;
            m.executar(p)?
        };
        use crate::builtins::assincronos as aj;
        let ajudantes: [(&'static str, crate::funcoes::FnNativa); 11] = [
            ("e_construtor", aj::fa_e_construtor),
            ("e_objeto", aj::fa_e_objeto),
            ("e_promessa", js_e_promessa),
            ("definir", aj::fa_definir),
            ("chamar", aj::fa_chamar),
            ("erro_tipo", js_erro_tipo),
            ("erro_agregado", js_erro_agregado),
            ("especie", js_especie),
            ("chaves", js_chaves),
            ("enumeravel", js_enumeravel),
            ("objeto_nulo", js_objeto_nulo),
        ];
        let mut a: Vec<Valor> = ajudantes.iter().map(|&(n, f)| m.nativa(n, 0, f, false)).collect();
        a.push(m.chave_para_valor(ChaveProp::Nome(crate::intrinsecos::simb::ITERATOR)));
        m.intr.promessa_js = m.chamar(fabrica, Valor::UNDEFINED, &a)?;
    }
    let l = m.intr.promessa_js;
    let f = m.obter_indice(l, u64::from(i))?;
    m.chamar(f, Valor::UNDEFINED, args)
}

fn js_e_promessa(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(m.e_promessa(m.arg(a, 0))))
}

fn js_erro_tipo(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let msg = m.para_texto_rust(m.arg(a, 0))?;
    Ok(m.erro(crate::intrinsecos::TipoErro::Tipo, &msg))
}

fn js_erro_agregado(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let e = m.erro(crate::intrinsecos::TipoErro::Agregado, "All promises were rejected");
    let l = m.arg(a, 0);
    m.definir_propria(e, fixos::ERRORS, l, attr::METODO);
    Ok(e)
}

fn js_especie(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    especie(m, m.arg(a, 0))
}

fn js_chaves(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let ks = m.chaves_de(m.arg(a, 0), false, true, true);
    let vs: Vec<Valor> = ks.into_iter().map(|k| m.chave_para_valor(k)).collect();
    Ok(m.array_de(&vs))
}

fn js_enumeravel(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let k = m.para_chave(m.arg(a, 1))?;
    let d = m.descritor_proprio(m.arg(a, 0), k);
    Ok(Valor::bool(d.is_some_and(|d| d.attrs & attr::ENUMERAVEL != 0)))
}

fn js_objeto_nulo(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.forma_para(Classe::Comum, Valor::NULL, 0);
    Ok(m.novo_objeto(f))
}

fn finally(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_FINALLY, &[this, m.arg(a, 0)])
}

fn resolve(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    if this == m.intr.promessa_ctor {
        return m.promessa_resolve(v);
    }
    chamar_js(m, JS_RESOLVE, &[this, v])
}

fn reject(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_REJECT, &[this, m.arg(a, 0)])
}

fn with_resolvers(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_WITH_RESOLVERS, &[this])
}

fn try_(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let resto: Vec<Valor> = (1..a.n).map(|i| m.arg(a, i)).collect();
    let resto = m.array_de(&resto);
    chamar_js(m, JS_TRY, &[this, m.arg(a, 0), resto])
}

fn all(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_ALL, &[this, m.arg(a, 0)])
}

fn all_settled(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_ALL_SETTLED, &[this, m.arg(a, 0)])
}

fn any(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_ANY, &[this, m.arg(a, 0)])
}

fn race(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_RACE, &[this, m.arg(a, 0)])
}

fn all_keyed(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_ALL_KEYED, &[this, m.arg(a, 0)])
}

fn all_settled_keyed(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    chamar_js(m, JS_ALL_SETTLED_KEYED, &[this, m.arg(a, 0)])
}

fn queue_microtask(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return m.erro_tipo("queueMicrotask: o argumento não é uma função");
    }
    m.microtarefas.push_back(Tarefa::Funcao { f });
    Ok(Valor::UNDEFINED)
}
