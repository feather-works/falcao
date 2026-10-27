//! Geradores async (§27.6), o iterador async que embrulha um comum (§27.1.6) e os construtores
//! GeneratorFunction, AsyncFunction e AsyncGeneratorFunction.
//!
//! Gerador async: campo 0 = quadro suspenso (cru), campo 1 = fila `[estado, (tipo, valor,
//! promessa)…]`, campo 2 = função. Cada next/return/throw entra na fila com a promessa que
//! devolve; o corpo roda até um `yield` (resolve o primeiro pedido) ou um `await`.

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::interpretador::{Entrada, Fim, AGUARDA, RENDEU};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

/// Estados do gerador async (item 0 da fila).
pub(crate) const AG_INICIO: i32 = 0;
const AG_PARADO: i32 = 1;
const AG_RODANDO: i32 = 2;
const AG_ESPERA_RETORNO: i32 = 3;
const AG_FIM: i32 = 4;

/// Tipos de pedido na fila.
const PEDE_NEXT: i32 = 0;
const PEDE_THROW: i32 = 1;
const PEDE_RETURN: i32 = 2;

/// Marca (campo 2 cru) do iterador async que embrulha um comum.
const MARCA_DE_SYNC: u32 = 0xA5_1C;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;
    let fun = {
        let c = m.chave("Function");
        m.obter(g, ChaveProp::Nome(c)).unwrap_or(Valor::UNDEFINED)
    };
    let tag = |m: &mut Motor, o: Valor, nome: &str| {
        let t = m.texto(nome);
        m.definir_propria(o, simb::TO_STRING_TAG, t, attr::CONFIGURAVEL);
    };
    // Construtor (fora do global) ligado ao protótipo das funções do tipo.
    let construtor = |m: &mut Motor, nome: &'static str, f: crate::funcoes::FnNativa, proto: Valor| {
        let c = m.nativa(nome, 1, f, true);
        if m.e_objeto(fun) {
            m.definir_proto(c, fun);
        }
        m.definir_propria(c, fixos::PROTOTYPE, proto, 0);
        m.definir_propria(proto, fixos::CONSTRUCTOR, c, attr::CONFIGURAVEL);
    };

    let (gfp, gp) = (m.intr.gerador_funcao_proto, m.intr.gerador_proto);
    construtor(m, "GeneratorFunction", construtor_gerador, gfp);
    tag(m, gfp, "GeneratorFunction");
    m.definir_propria(gp, fixos::CONSTRUCTOR, gfp, attr::CONFIGURAVEL);
    tag(m, gp, "Generator");

    let afp = m.intr.async_funcao_proto;
    construtor(m, "AsyncFunction", construtor_async, afp);
    tag(m, afp, "AsyncFunction");

    let (agfp, agp) = (m.intr.async_gerador_funcao_proto, m.intr.async_gerador_proto);
    construtor(m, "AsyncGeneratorFunction", construtor_gerador_async, agfp);
    tag(m, agfp, "AsyncGeneratorFunction");
    m.definir_propria(agfp, fixos::PROTOTYPE, agp, attr::CONFIGURAVEL);
    m.definir_propria(agp, fixos::CONSTRUCTOR, agfp, attr::CONFIGURAVEL);
    tag(m, agp, "AsyncGenerator");
    m.metodo(agp, "next", 1, ag_next);
    m.metodo(agp, "return", 1, ag_return);
    m.metodo(agp, "throw", 1, ag_throw);

    let aip = m.intr.async_iterador_proto;
    let ai = m.nativa("[Symbol.asyncIterator]", 0, devolve_this, false);
    m.definir_propria(aip, simb::ASYNC_ITERATOR, ai, attr::METODO);

    let ads = m.intr.async_de_sync_proto;
    m.metodo(ads, "next", 1, ads_next);
    m.metodo(ads, "return", 1, ads_return);
    m.metodo(ads, "throw", 1, ads_throw);

    let c = m.chave("Array");
    if let Ok(array) = m.obter(g, ChaveProp::Nome(c)) {
        m.metodo(array, "fromAsync", 1, from_async);
    }
}

// ---------------------------------------------------------------- Array.fromAsync

/// O corpo é uma função async em JS (ela espera cada item); só ajudantes nativos e intrínsecos
/// capturados aqui, então mexer no global depois não muda o comportamento.
const FROM_ASYNC: &str = r#"(function (eConstrutor, registro, eObjeto, definir, paraObjeto, comprimento, criarArray, chamar, ErroDeTipo) {
'use strict';
async function fechar(it) {
  try { const f = it.return; if (f !== undefined && f !== null) await chamar(f, it, []); } catch (_) {}
}
return async function fromAsync(itens, mapfn, thisArg) {
  const C = this;
  if (mapfn !== undefined && typeof mapfn !== 'function') throw new ErroDeTipo('Array.fromAsync: mapfn is not callable');
  const mapear = mapfn !== undefined;
  const reg = registro(itens);
  if (reg !== undefined) {
    const A = eConstrutor(C) ? new C() : criarArray(0);
    const it = reg[0], proximo = reg[1];
    for (let k = 0; ; k++) {
      const r = await chamar(proximo, it, []);
      if (!eObjeto(r)) throw new ErroDeTipo('Iterator result is not an object');
      if (r.done) { A.length = k; return A; }
      let v = r.value;
      if (mapear) {
        try { v = await chamar(mapfn, thisArg, [v, k]); } catch (e) { await fechar(it); throw e; }
      }
      try { definir(A, k, v); } catch (e) { await fechar(it); throw e; }
    }
  }
  const lista = paraObjeto(itens);
  const n = comprimento(lista);
  const A = eConstrutor(C) ? new C(n) : criarArray(n);
  for (let k = 0; k < n; k++) {
    let v = await lista[k];
    if (mapear) v = await chamar(mapfn, thisArg, [v, k]);
    definir(A, k, v);
  }
  A.length = n;
  return A;
};
})"#;

fn from_async(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let eu = m.chamado;
    let mut alvo = m.capturado();
    if alvo == Valor::UNDEFINED {
        let fabrica = {
            let p = m.compilar_texto(FROM_ASYNC)?;
            m.executar(p)?
        };
        let erro_tipo = {
            let g = m.intr.global;
            let c = m.chave("TypeError");
            m.obter(g, ChaveProp::Nome(c))?
        };
        let ajudantes: [(&'static str, crate::funcoes::FnNativa); 8] = [
            ("e_construtor", fa_e_construtor),
            ("registro", fa_registro),
            ("e_objeto", fa_e_objeto),
            ("definir", fa_definir),
            ("para_objeto", fa_para_objeto),
            ("comprimento", fa_comprimento),
            ("criar_array", fa_criar_array),
            ("chamar", fa_chamar),
        ];
        let mut args: Vec<Valor> = ajudantes.iter().map(|&(n, f)| m.nativa(n, 0, f, false)).collect();
        args.push(erro_tipo);
        alvo = m.chamar(fabrica, Valor::UNDEFINED, &args)?;
        m.por_interno_valor(eu, crate::funcoes::F_CONTEXTO, alvo);
    }
    let args: Vec<Valor> = (0..a.n).map(|i| m.arg(a, i)).collect();
    m.chamar(alvo, this, &args)
}

pub(crate) fn fa_e_construtor(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(m.e_construtor(m.arg(a, 0))))
}

/// `[iterador, next]` pelo `[Symbol.asyncIterator]` ou pelo comum embrulhado; undefined se não é
/// iterável (é array-like).
fn fa_registro(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let (it, next) = match m.obter_metodo(v, ChaveProp::Nome(simb::ASYNC_ITERATOR))? {
        Some(f) => {
            let it = m.chamar(f, v, &[])?;
            if !m.e_objeto(it) {
                return m.erro_tipo("Result of the Symbol.asyncIterator method is not an object");
            }
            let next = m.obter(it, ChaveProp::Nome(fixos::NEXT))?;
            (it, next)
        }
        None => {
            if m.obter_metodo(v, ChaveProp::Nome(simb::ITERATOR))?.is_none() {
                return Ok(Valor::UNDEFINED);
            }
            let reg = m.obter_iterador_real(v)?;
            let w = m.async_de_sync(reg);
            let next = m.obter(w, ChaveProp::Nome(fixos::NEXT))?;
            (w, next)
        }
    };
    Ok(m.array_de(&[it, next]))
}

pub(crate) fn fa_e_objeto(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(m.e_objeto(m.arg(a, 0))))
}

/// CreateDataPropertyOrThrow(A, k, v).
pub(crate) fn fa_definir(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (o, k, v) = (m.arg(a, 0), m.arg(a, 1), m.arg(a, 2));
    let k = m.para_chave(k)?;
    if !m.criar_dado(o, k, v)? {
        return m.erro_tipo("Cannot define property on the result of Array.fromAsync");
    }
    Ok(Valor::UNDEFINED)
}

fn fa_para_objeto(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    m.para_objeto(m.arg(a, 0))
}

fn fa_comprimento(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let n = m.comprimento_de(m.arg(a, 0))?;
    Ok(m.numero(n as f64))
}

/// ArrayCreate(n): RangeError acima de 2^32 - 1.
fn fa_criar_array(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let n = m.para_numero(m.arg(a, 0))?;
    if n > f64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
    let arr = m.array_de(&[]);
    m.definir_array_len(arr, n as u32);
    Ok(arr)
}

/// Reflect.apply(f, this, [args]) sem passar pelo `Reflect` do global.
pub(crate) fn fa_chamar(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (f, t, l) = (m.arg(a, 0), m.arg(a, 1), m.arg(a, 2));
    let n = m.comprimento_de(l)?;
    let mut args = Vec::with_capacity(n as usize);
    for i in 0..n {
        args.push(m.obter_indice(l, i)?);
    }
    m.chamar(f, t, &args)
}

fn devolve_this(_m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(this)
}

// ---------------------------------------------------------------- construtores dinâmicos

/// CreateDynamicFunction para `function*`, `async function` e `async function*`.
fn dinamica(m: &mut Motor, a: Args, nt: Valor, prefixo: &str) -> Res<Valor> {
    let mut params = Vec::new();
    for i in 0..a.n.saturating_sub(1) {
        params.push(m.para_texto_rust(m.arg(a, i))?);
    }
    let corpo = if a.n > 0 { m.para_texto_rust(m.arg(a, a.n - 1))? } else { String::new() };
    let fonte = format!("({prefixo} anonymous({}\n) {{\n{}\n}})", params.join(","), corpo);
    let p = m.compilar_texto(&fonte)?;
    let f = m.executar(p)?;
    // Subclasse (`new class extends AsyncFunction {}`): o protótipo vem do novo alvo.
    if nt != Valor::UNDEFINED && nt != m.chamado {
        let pr = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
        if m.e_objeto(pr) {
            m.definir_proto(f, pr);
        }
    }
    Ok(f)
}

fn construtor_gerador(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    dinamica(m, a, nt, "function*")
}

fn construtor_async(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    dinamica(m, a, nt, "async function")
}

fn construtor_gerador_async(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    dinamica(m, a, nt, "async function*")
}

// ---------------------------------------------------------------- gerador async

impl Motor {
    pub(crate) fn e_gerador_async(&self, v: Valor) -> bool {
        self.e_objeto(v) && self.formas.get(self.forma(v)).classe == Classe::Gerador && Valor(self.interno(v, 1)) != Valor::UNDEFINED
    }

    fn ag_fila(&self, g: Valor) -> Valor {
        Valor(self.interno(g, 1))
    }

    fn ag_estado(&self, g: Valor) -> i32 {
        self.lista_get(self.ag_fila(g), 0).como_int()
    }

    fn ag_por_estado(&mut self, g: Valor, e: i32) {
        let f = self.ag_fila(g);
        self.lista_set(f, 0, Valor::int(e));
    }

    fn ag_enfileirar(&mut self, g: Valor, tipo: i32, v: Valor, p: Valor) {
        let f = self.ag_fila(g);
        let n = self.lista_len(f);
        let nova = self.redimensionar_lista(f, n + 3, Valor::UNDEFINED);
        self.lista_set(nova, n, Valor::int(tipo));
        self.lista_set(nova, n + 1, v);
        self.lista_set(nova, n + 2, p);
        self.por_interno_valor(g, 1, nova);
    }

    fn ag_primeiro(&self, g: Valor) -> Option<(i32, Valor)> {
        let f = self.ag_fila(g);
        (self.lista_len(f) >= 4).then(|| (self.lista_get(f, 1).como_int(), self.lista_get(f, 2)))
    }

    /// Tira o primeiro pedido da fila; devolve a promessa dele.
    fn ag_tirar(&mut self, g: Valor) -> Valor {
        let f = self.ag_fila(g);
        let n = self.lista_len(f);
        let p = self.lista_get(f, 3);
        let nova = self.nova_lista(n - 3);
        let f = self.ag_fila(g);
        self.lista_set(nova, 0, self.lista_get(f, 0));
        for i in 4..n {
            let x = self.lista_get(f, i);
            self.lista_set(nova, i - 3, x);
        }
        self.por_interno_valor(g, 1, nova);
        p
    }

    /// AsyncGeneratorCompleteStep: assenta a promessa do primeiro pedido.
    fn ag_completar(&mut self, g: Valor, lanca: bool, v: Valor, feito: bool) {
        let p = self.ag_tirar(g);
        if lanca {
            self.rejeitar_promessa(p, v);
        } else {
            let r = self.resultado_iterador(v, feito);
            self.resolver_promessa(p, r);
        }
    }

    /// Solta o quadro parado (o corpo não roda mais).
    fn ag_soltar_quadro(&mut self, g: Valor) {
        let id = self.interno(g, 0);
        if id != u32::MAX {
            self.liberar_suspenso(id);
            self.por_interno(g, 0, u32::MAX);
        }
    }

    fn ag_retomar(&mut self, g: Valor, ent: Entrada) {
        self.ag_por_estado(g, AG_RODANDO);
        let id = self.interno(g, 0);
        let r = self.retomar(id, ent);
        self.depois_async(id, r);
    }

    /// Depois que o corpo parou (`yield`, `await`), acabou ou lançou.
    pub(crate) fn depois_gerador_async(&mut self, g: Valor, id: u32, r: Res<Fim>) {
        let (mut g, mut r) = (g, r);
        loop {
            match r {
                Ok(Fim::Suspenso(v)) => {
                    let modo = self.suspensos[id as usize].as_ref().map_or(RENDEU, |s| s.modo);
                    if let Some(s) = self.suspensos[id as usize].as_mut() {
                        s.promessa = g;
                    }
                    if modo == AGUARDA {
                        return self.aguardar(id, v);
                    }
                    self.ag_completar(g, false, v, false);
                    // Já tem pedido na fila: segue sem parar (AsyncGeneratorYield).
                    let Some((tipo, x)) = self.ag_primeiro(g) else {
                        self.ag_por_estado(g, AG_PARADO);
                        return;
                    };
                    r = self.retomar(id, entrada(tipo, x));
                    g = self.promessas_async.remove(&id).unwrap_or(g);
                }
                Ok(Fim::Retorno(v)) => {
                    self.ag_soltar_quadro(g);
                    self.ag_por_estado(g, AG_FIM);
                    self.ag_completar(g, false, v, true);
                    return self.ag_drenar(g);
                }
                Err(e) => {
                    self.ag_soltar_quadro(g);
                    self.ag_por_estado(g, AG_FIM);
                    self.ag_completar(g, true, e, true);
                    return self.ag_drenar(g);
                }
            }
        }
    }

    /// AsyncGeneratorDrainQueue (o gerador acabou).
    fn ag_drenar(&mut self, g: Valor) {
        while let Some((tipo, x)) = self.ag_primeiro(g) {
            match tipo {
                PEDE_RETURN => {
                    self.ag_por_estado(g, AG_ESPERA_RETORNO);
                    return self.ag_aguardar_retorno(g);
                }
                PEDE_THROW => self.ag_completar(g, true, x, true),
                _ => self.ag_completar(g, false, Valor::UNDEFINED, true),
            }
        }
    }

    /// AsyncGeneratorAwaitReturn: o primeiro pedido é `return(v)`; espera v e acaba.
    fn ag_aguardar_retorno(&mut self, g: Valor) {
        let Some((_, v)) = self.ag_primeiro(g) else { return };
        let p = match self.promessa_resolve(v) {
            Ok(p) => p,
            Err(e) => {
                self.ag_por_estado(g, AG_FIM);
                self.ag_completar(g, true, e, true);
                return self.ag_drenar(g);
            }
        };
        let i_ok = self.id_nativa("ag_retorno_ok", ag_retorno_ok);
        let i_err = self.id_nativa("ag_retorno_err", ag_retorno_err);
        let ok = self.nativa_com(i_ok, 1, g);
        let err = self.nativa_com(i_err, 1, g);
        self.entao(p, ok, err);
    }

    /// PromiseResolve(%Promise%, v): a própria se é promessa cujo `constructor` é o Promise.
    pub(crate) fn promessa_resolve(&mut self, v: Valor) -> Res<Valor> {
        if self.e_promessa(v) {
            if self.obter(v, ChaveProp::Nome(fixos::CONSTRUCTOR))? == self.intr.promessa_ctor {
                return Ok(v);
            }
        }
        let p = self.nova_promessa();
        self.resolver_promessa(p, v);
        Ok(p)
    }

    /// CreateAsyncFromSyncIterator: embrulha o registro de iterador comum.
    pub(crate) fn async_de_sync(&mut self, reg: Valor) -> Valor {
        let p = self.intr.async_de_sync_proto;
        let f = self.forma_para(Classe::Iterador, p, 0);
        let w = self.novo_objeto(f);
        self.por_interno_valor(w, 0, reg);
        self.por_interno(w, 2, MARCA_DE_SYNC);
        w
    }
}

fn entrada(tipo: i32, v: Valor) -> Entrada {
    match tipo {
        PEDE_THROW => Entrada::Lanca(v),
        PEDE_RETURN => Entrada::Retorna(v),
        _ => Entrada::Normal(v),
    }
}

fn ag_retorno_ok(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let g = m.capturado();
    m.ag_por_estado(g, AG_FIM);
    m.ag_completar(g, false, m.arg(a, 0), true);
    m.ag_drenar(g);
    Ok(Valor::UNDEFINED)
}

fn ag_retorno_err(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let g = m.capturado();
    m.ag_por_estado(g, AG_FIM);
    m.ag_completar(g, true, m.arg(a, 0), true);
    m.ag_drenar(g);
    Ok(Valor::UNDEFINED)
}

fn rejeitada(m: &mut Motor, e: Valor) -> Valor {
    let p = m.nova_promessa();
    m.rejeitar_promessa(p, e);
    p
}

/// AsyncGeneratorValidate: erro vira promessa rejeitada (os métodos nunca lançam).
fn validar(m: &mut Motor, this: Valor, metodo: &str) -> Result<(), Valor> {
    if m.e_gerador_async(this) {
        return Ok(());
    }
    Err(m.erro(TipoErro::Tipo, &format!("AsyncGenerator.prototype.{metodo} called on incompatible receiver")))
}

fn ag_next(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if let Err(e) = validar(m, this, "next") {
        return Ok(rejeitada(m, e));
    }
    let p = m.nova_promessa();
    let v = m.arg(a, 0);
    let st = m.ag_estado(this);
    if st == AG_FIM {
        let r = m.resultado_iterador(Valor::UNDEFINED, true);
        m.resolver_promessa(p, r);
        return Ok(p);
    }
    m.ag_enfileirar(this, PEDE_NEXT, v, p);
    if st == AG_INICIO || st == AG_PARADO {
        m.ag_retomar(this, Entrada::Normal(v));
    }
    Ok(p)
}

fn ag_return(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if let Err(e) = validar(m, this, "return") {
        return Ok(rejeitada(m, e));
    }
    let p = m.nova_promessa();
    let v = m.arg(a, 0);
    let st = m.ag_estado(this);
    m.ag_enfileirar(this, PEDE_RETURN, v, p);
    match st {
        AG_INICIO | AG_FIM => {
            m.ag_soltar_quadro(this);
            m.ag_por_estado(this, AG_ESPERA_RETORNO);
            m.ag_aguardar_retorno(this);
        }
        AG_PARADO => m.ag_retomar(this, Entrada::Retorna(v)),
        _ => {}
    }
    Ok(p)
}

fn ag_throw(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if let Err(e) = validar(m, this, "throw") {
        return Ok(rejeitada(m, e));
    }
    let p = m.nova_promessa();
    let e = m.arg(a, 0);
    let mut st = m.ag_estado(this);
    if st == AG_INICIO {
        m.ag_soltar_quadro(this);
        m.ag_por_estado(this, AG_FIM);
        st = AG_FIM;
    }
    if st == AG_FIM {
        m.rejeitar_promessa(p, e);
        return Ok(p);
    }
    m.ag_enfileirar(this, PEDE_THROW, e, p);
    if st == AG_PARADO {
        m.ag_retomar(this, Entrada::Lanca(e));
    }
    Ok(p)
}

// ---------------------------------------------------------------- %AsyncFromSyncIteratorPrototype%

/// O registro do iterador comum embrulhado.
fn registro_de_sync(m: &mut Motor, this: Valor) -> Result<Valor, Valor> {
    if m.e_objeto(this) && m.formas.get(m.forma(this)).classe == Classe::Iterador && m.interno(this, 2) == MARCA_DE_SYNC {
        return Ok(Valor(m.interno(this, 0)));
    }
    Err(m.erro(TipoErro::Tipo, "not an async-from-sync iterator"))
}

/// Argumentos repassados: o valor só se veio (`next()` e `next(undefined)` diferem no `arguments`).
fn so_se_veio(m: &Motor, a: Args) -> Vec<Valor> {
    if a.n > 0 {
        vec![m.arg(a, 0)]
    } else {
        Vec::new()
    }
}

fn ads_next(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let reg = match registro_de_sync(m, this) {
        Ok(r) => r,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    let (it, next) = (m.lista_get(reg, 0), m.lista_get(reg, 1));
    let args = so_se_veio(m, a);
    let r = match m.chamar(next, it, &args) {
        Ok(r) => r,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    if !m.e_objeto(r) {
        let e = m.erro(TipoErro::Tipo, "Iterator result is not an object");
        return Ok(rejeitada(m, e));
    }
    Ok(continuacao(m, r, it, true))
}

fn ads_return(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let reg = match registro_de_sync(m, this) {
        Ok(r) => r,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    let it = m.lista_get(reg, 0);
    let f = match m.obter_metodo(it, ChaveProp::Nome(fixos::RETURN)) {
        Ok(f) => f,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    let Some(f) = f else {
        let r = m.resultado_iterador(m.arg(a, 0), true);
        let p = m.nova_promessa();
        m.resolver_promessa(p, r);
        return Ok(p);
    };
    let args = so_se_veio(m, a);
    let r = match m.chamar(f, it, &args) {
        Ok(r) => r,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    if !m.e_objeto(r) {
        let e = m.erro(TipoErro::Tipo, "Iterator result is not an object");
        return Ok(rejeitada(m, e));
    }
    Ok(continuacao(m, r, it, false))
}

fn ads_throw(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let reg = match registro_de_sync(m, this) {
        Ok(r) => r,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    let it = m.lista_get(reg, 0);
    let f = match m.obter_metodo(it, ChaveProp::Nome(fixos::THROW)) {
        Ok(f) => f,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    let Some(f) = f else {
        // Sem `throw`: fecha o de dentro (erro do fechamento vence) e rejeita com TypeError.
        let fechou = (|| {
            if let Some(r) = m.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))? {
                let x = m.chamar(r, it, &[])?;
                if !m.e_objeto(x) {
                    return m.erro_tipo("Iterator result is not an object");
                }
            }
            Ok(())
        })();
        let e = match fechou {
            Err(e) => e,
            Ok(()) => m.erro(TipoErro::Tipo, "The iterator does not provide a 'throw' method"),
        };
        return Ok(rejeitada(m, e));
    };
    let r = match m.chamar(f, it, &[m.arg(a, 0)]) {
        Ok(r) => r,
        Err(e) => return Ok(rejeitada(m, e)),
    };
    if !m.e_objeto(r) {
        let e = m.erro(TipoErro::Tipo, "Iterator result is not an object");
        return Ok(rejeitada(m, e));
    }
    Ok(continuacao(m, r, it, true))
}

/// AsyncFromSyncIteratorContinuation: espera o valor do resultado; se ele rejeitar e o
/// iterador não acabou, fecha o iterador comum.
fn continuacao(m: &mut Motor, r: Valor, it: Valor, fechar: bool) -> Valor {
    let feito = match m.obter(r, ChaveProp::Nome(fixos::DONE)) {
        Ok(d) => m.verdadeiro(d),
        Err(e) => return rejeitada(m, e),
    };
    let v = match m.obter(r, ChaveProp::Nome(fixos::VALUE)) {
        Ok(v) => v,
        Err(e) => return rejeitada(m, e),
    };
    let embrulho = match m.promessa_resolve(v) {
        Ok(p) => p,
        Err(e) => {
            if !feito && fechar {
                fechar_engolindo(m, it);
            }
            return rejeitada(m, e);
        }
    };
    let i_ok = m.id_nativa("ads_desembrulha", ads_desembrulha);
    let ok = m.nativa_com(i_ok, 1, Valor::bool(feito));
    let err = if feito || !fechar {
        Valor::UNDEFINED
    } else {
        let i_err = m.id_nativa("ads_fecha", ads_fecha);
        m.nativa_com(i_err, 1, it)
    };
    m.entao(embrulho, ok, err)
}

/// IteratorClose com conclusão de erro: o erro original vence.
fn fechar_engolindo(m: &mut Motor, it: Valor) {
    if let Ok(Some(f)) = m.obter_metodo(it, ChaveProp::Nome(fixos::RETURN)) {
        let _ = m.chamar(f, it, &[]);
    }
}

fn ads_desembrulha(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let feito = m.capturado() == Valor::TRUE;
    Ok(m.resultado_iterador(m.arg(a, 0), feito))
}

fn ads_fecha(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let it = m.capturado();
    fechar_engolindo(m, it);
    Err(m.arg(a, 0))
}

#[cfg(test)]
mod testes {
    use crate::motor::Motor;

    #[test]
    fn gerador_async_fila_e_for_await() {
        let mut m = Motor::novo();
        let saida = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let s2 = saida.clone();
        m.ao_imprimir = Some(Box::new(move |_, t| s2.borrow_mut().push(t.to_owned())));
        m.avaliar(
            "async function* g(){ try { var x = yield 1; console.log('x', x); yield await Promise.resolve(2); } finally { console.log('fim'); } }
             var it = g(); it.next().then(r => console.log(r.value, r.done)); it.next('a').then(r => console.log(r.value, r.done));
             it.return(9).then(r => console.log(r.value, r.done));
             (async () => { for await (const v of [Promise.resolve(10), 20]) console.log('fa', v); })();",
        )
        .unwrap();
        m.rodar_microtarefas();
        // A mesma ordem do SpiderMonkey 157.
        assert_eq!(*saida.borrow(), ["x a", "1 false", "fa 10", "2 false", "fim", "fa 20", "9 true"]);
    }

    #[test]
    fn promessa_combinadores_e_especie() {
        let mut m = Motor::novo();
        let saida = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let s2 = saida.clone();
        m.ao_imprimir = Some(Box::new(move |_, t| s2.borrow_mut().push(t.to_owned())));
        m.avaliar(
            "Promise.all([1, Promise.resolve(2)]).then(v => console.log('all', String(v)));
             Promise.any([Promise.reject(1)]).catch(e => console.log(e.constructor.name, String(e.errors)));
             new Promise((r, j) => { r({ then(ok) { ok(3); } }); throw 0; }).then(v => console.log('fica', v));
             class P extends Promise {}
             console.log(P.resolve(1).then(x => x) instanceof P, P.race([]) instanceof P);
             Promise.resolve(5).finally(() => {}).then(v => console.log('fin', v));",
        )
        .unwrap();
        m.rodar_microtarefas();
        // A mesma ordem do SpiderMonkey.
        assert_eq!(*saida.borrow(), ["true true", "all 1,2", "AggregateError 1", "fica 3", "fin 5"]);
    }
}
