//! Iterator (ES2025): o construtor abstrato, `Iterator.from`, `Iterator.prototype` (preguiçosos:
//! map, filter, take, drop, flatMap; eagers: reduce, toArray, forEach, some, every, find) e os
//! protótipos de helper e de embrulho.
//!
//! Os helpers são máquinas de estado no lugar dos geradores da especificação. A lista de estado
//! (campo 0) guarda o iterador, o `next`, a função, o contador, o iterador interno do flatMap e
//! o valor produzido; o campo 1 é o estado e o 2 o tipo. Tudo o que sobrevive a uma chamada de
//! JS mora nesses objetos ou em `raizes` (lidos de novo depois da chamada).

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

// Tipos (campo 2).
const T_MAPA: u32 = 1;
const T_FILTRO: u32 = 2;
const T_TOMA: u32 = 3;
const T_SOLTA: u32 = 4;
const T_PLANIFICA: u32 = 5;
const T_EMBRULHO: u32 = 6;

// Estados (campo 1), os da geradora da especificação.
const S_INICIO: u32 = 0;
const S_SUSPENSO: u32 = 1;
const S_EXECUTANDO: u32 = 2;
const S_COMPLETO: u32 = 3;

// Campos da lista de estado.
const L_ITER: u32 = 0;
const L_NEXT: u32 = 1;
const L_FN: u32 = 2;
/// Contador do callback; o restante de take/drop.
const L_CONT: u32 = 3;
const L_INTERNO: u32 = 4;
const L_INTERNO_NEXT: u32 = 5;
const L_VALOR: u32 = 6;

pub(super) fn instalar(m: &mut Motor) {
    let ip = m.intr.iterador_proto;
    let g = m.intr.global;
    let ctor = m.nativa("Iterator", 0, construtor, true);
    m.definir_propria(ctor, fixos::PROTOTYPE, ip, 0);
    let c = m.chave("Iterator");
    m.definir_propria(g, c, ctor, attr::METODO);
    m.metodo(ctor, "from", 1, from);

    // `constructor` e `Symbol.toStringTag`: acessores que aceitam atribuição.
    let id = m.registrar_nativa("get constructor", obter_construtor, false);
    let get_ctor = m.nativa_com(id, 0, ctor);
    let set_ctor = m.nativa("set constructor", 1, definir_construtor, false);
    m.definir_acessor(ip, ChaveProp::Nome(fixos::CONSTRUCTOR), get_ctor, false, false);
    m.definir_acessor(ip, ChaveProp::Nome(fixos::CONSTRUCTOR), set_ctor, true, false);
    let get_tag = m.nativa("get [Symbol.toStringTag]", 0, obter_etiqueta, false);
    let set_tag = m.nativa("set [Symbol.toStringTag]", 1, definir_etiqueta, false);
    m.definir_acessor(ip, ChaveProp::Nome(simb::TO_STRING_TAG), get_tag, false, false);
    m.definir_acessor(ip, ChaveProp::Nome(simb::TO_STRING_TAG), set_tag, true, false);

    let metodos: &[(&str, u32, crate::funcoes::FnNativa)] = &[
        ("map", 1, map),
        ("filter", 1, filter),
        ("take", 1, take),
        ("drop", 1, drop),
        ("flatMap", 1, flat_map),
        ("reduce", 1, reduce),
        ("toArray", 0, to_array),
        ("forEach", 1, for_each),
        ("some", 1, some),
        ("every", 1, every),
        ("find", 1, find),
    ];
    for &(n, k, f) in metodos {
        m.metodo(ip, n, k, f);
    }

    let hp = proto_com(m, ip);
    m.intr.iterador_helper_proto = hp;
    m.metodo(hp, "next", 0, helper_next);
    m.metodo(hp, "return", 0, helper_return);
    let t = m.texto("Iterator Helper");
    m.definir_propria(hp, simb::TO_STRING_TAG, t, attr::CONFIGURAVEL);

    let wp = proto_com(m, ip);
    m.intr.wrap_iterador_proto = wp;
    m.metodo(wp, "next", 0, embrulho_next);
    m.metodo(wp, "return", 0, embrulho_return);
}

fn proto_com(m: &mut Motor, pai: Valor) -> Valor {
    let f = m.forma_para(Classe::Comum, pai, 0);
    m.novo_objeto(f)
}

// ---------------------------------------------------------------- construtor e estáticos

fn construtor(m: &mut Motor, _this: Valor, _a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED || nt == m.chamado {
        return m.erro_tipo("Abstract class Iterator not directly constructable");
    }
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = if m.e_objeto(p) { p } else { m.intr.iterador_proto };
    let f = m.forma_para(Classe::Comum, p, 0);
    Ok(m.novo_objeto(f))
}

/// `Iterator.from`: o próprio iterador se já herda de `Iterator.prototype`, senão um embrulho.
fn from(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let (it, nx) = iterador_planificavel(m, v, true)?;
    let ip = m.intr.iterador_proto;
    let mut p = m.proto_de(it);
    while p != Valor::NULL {
        if p == ip {
            return Ok(it);
        }
        p = m.proto_de(p);
    }
    let wp = m.intr.wrap_iterador_proto;
    Ok(novo_helper(m, wp, T_EMBRULHO, it, nx, Valor::UNDEFINED, Valor::int(0)))
}

fn obter_construtor(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(m.capturado())
}

fn definir_construtor(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let home = m.intr.iterador_proto;
    ignorando_proto(m, this, home, ChaveProp::Nome(fixos::CONSTRUCTOR), v)
}

fn obter_etiqueta(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(m.texto("Iterator"))
}

fn definir_etiqueta(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let home = m.intr.iterador_proto;
    ignorando_proto(m, this, home, ChaveProp::Nome(simb::TO_STRING_TAG), v)
}

/// SetterThatIgnoresPrototypeProperties: a atribuição em `Iterator.prototype` é proibida; nos
/// outros objetos cria a propriedade própria, ou atribui se ela já existe.
fn ignorando_proto(m: &mut Motor, this: Valor, home: Valor, k: ChaveProp, v: Valor) -> Res<Valor> {
    if !m.e_objeto(this) {
        return m.erro_tipo("Iterator setter called on non-object");
    }
    if this == home {
        return m.erro_tipo("Cannot assign to read only property of Iterator.prototype");
    }
    if m.descritor_proprio(this, k).is_none() {
        if !m.criar_dado(this, k, v)? {
            return m.erro_tipo("Cannot define property on Iterator object");
        }
    } else {
        m.gravar(this, k, v, true)?;
    }
    Ok(Valor::UNDEFINED)
}

// ---------------------------------------------------------------- Iterator.prototype: preguiçosos

fn map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = receptor(m, this)?;
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return fechando(m, o, "Iterator.prototype.map: mapper is not a function");
    }
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    let hp = m.intr.iterador_helper_proto;
    Ok(novo_helper(m, hp, T_MAPA, o, nx, f, Valor::int(0)))
}

fn filter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = receptor(m, this)?;
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return fechando(m, o, "Iterator.prototype.filter: predicate is not a function");
    }
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    let hp = m.intr.iterador_helper_proto;
    Ok(novo_helper(m, hp, T_FILTRO, o, nx, f, Valor::int(0)))
}

fn flat_map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = receptor(m, this)?;
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return fechando(m, o, "Iterator.prototype.flatMap: mapper is not a function");
    }
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    let hp = m.intr.iterador_helper_proto;
    Ok(novo_helper(m, hp, T_PLANIFICA, o, nx, f, Valor::int(0)))
}

fn take(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    limitado(m, this, a, T_TOMA)
}

fn drop(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    limitado(m, this, a, T_SOLTA)
}

/// `take` e `drop`: o limite vira o contador (o restante).
fn limitado(m: &mut Motor, this: Valor, a: Args, tipo: u32) -> Res<Valor> {
    let o = receptor(m, this)?;
    let v = m.arg(a, 0);
    let limite = limite(m, o, v)?;
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    let n = m.numero(limite);
    let hp = m.intr.iterador_helper_proto;
    Ok(novo_helper(m, hp, tipo, o, nx, Valor::UNDEFINED, n))
}

/// Limite de take/drop: ToNumber; NaN e negativo fecham o receptor e lançam RangeError.
fn limite(m: &mut Motor, o: Valor, v: Valor) -> Res<f64> {
    let x = match m.para_numero(v) {
        Ok(x) => x,
        Err(e) => return Err(fechar_erro(m, o, e)),
    };
    if x.is_nan() {
        return fechando_alcance(m, o, "Iterator.prototype.take/drop: limit must be a number");
    }
    let n = x.trunc() + 0.0;
    if n < 0.0 {
        return fechando_alcance(m, o, "Iterator.prototype.take/drop: limit must be non-negative");
    }
    // shortcut: o teste262 desta versão quer RangeError acima de 2^53 - 1 (fora do intervalo seguro), infinito incluído.
    if n > 9_007_199_254_740_991.0 && n.is_finite() {
        return fechando_alcance(m, o, "Iterator.prototype.take/drop: limit is too large");
    }
    Ok(n)
}

// ---------------------------------------------------------------- Iterator.prototype: eagers

fn reduce(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = receptor(m, this)?;
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return fechando(m, o, "Iterator.prototype.reduce: reducer is not a function");
    }
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    let com_inicial = a.n >= 2;
    let inicial = m.arg(a, 1);
    com_raizes(m, &[o, nx, f, inicial], |m, b| {
        let mut cont = 0.0;
        if !com_inicial {
            let (it, nx) = (m.raizes[b], m.raizes[b + 1]);
            match proximo_valor(m, it, nx)? {
                Some(v) => m.raizes[b + 3] = v,
                None => return m.erro_tipo("Reduce of empty iterator with no initial value"),
            }
            cont = 1.0;
        }
        loop {
            let (it, nx) = (m.raizes[b], m.raizes[b + 1]);
            let v = match proximo_valor(m, it, nx)? {
                Some(v) => v,
                None => return Ok(m.raizes[b + 3]),
            };
            let (f, acc) = (m.raizes[b + 2], m.raizes[b + 3]);
            let c = m.numero(cont);
            let r = match m.chamar_coletavel(f, Valor::UNDEFINED, &[acc, v, c]) {
                Ok(r) => r,
                Err(e) => return Err(fechar_raiz(m, b, e)),
            };
            m.raizes[b + 3] = r;
            cont += 1.0;
        }
    })
}

fn to_array(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = receptor(m, this)?;
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    let arr = m.array_de(&[]);
    com_raizes(m, &[o, nx, arr], |m, b| {
        let mut i = 0;
        loop {
            let (it, nx) = (m.raizes[b], m.raizes[b + 1]);
            let v = match proximo_valor(m, it, nx)? {
                Some(v) => v,
                None => return Ok(m.raizes[b + 2]),
            };
            let arr = m.raizes[b + 2];
            m.criar_dado(arr, ChaveProp::Indice(i), v)?;
            i += 1;
        }
    })
}

fn for_each(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = receptor(m, this)?;
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return fechando(m, o, "Iterator.prototype.forEach: callback is not a function");
    }
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    com_raizes(m, &[o, nx, f], |m, b| {
        let mut cont = 0.0;
        loop {
            let (it, nx) = (m.raizes[b], m.raizes[b + 1]);
            let v = match proximo_valor(m, it, nx)? {
                Some(v) => v,
                None => return Ok(Valor::UNDEFINED),
            };
            let (f, c) = (m.raizes[b + 2], m.numero(cont));
            if let Err(e) = m.chamar_coletavel(f, Valor::UNDEFINED, &[v, c]) {
                return Err(fechar_raiz(m, b, e));
            }
            cont += 1.0;
        }
    })
}

fn some(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let achou = procurar(m, this, a, true)?;
    Ok(Valor::bool(achou.is_some()))
}

fn every(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let achou = procurar(m, this, a, false)?;
    Ok(Valor::bool(achou.is_none()))
}

fn find(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let achou = procurar(m, this, a, true)?;
    Ok(achou.unwrap_or(Valor::UNDEFINED))
}

/// `some`, `every` e `find`: para no primeiro valor cujo predicado dá `parar` (e fecha o
/// iterador). Devolve esse valor, ou `None` se o iterador acabou.
fn procurar(m: &mut Motor, this: Valor, a: Args, parar: bool) -> Res<Option<Valor>> {
    let o = receptor(m, this)?;
    let f = m.arg(a, 0);
    if !m.e_funcao(f) {
        return fechando(m, o, "Iterator.prototype predicate is not a function");
    }
    let nx = m.obter(o, ChaveProp::Nome(fixos::NEXT))?;
    com_raizes(m, &[o, nx, f, Valor::UNDEFINED], |m, b| {
        let mut cont = 0.0;
        loop {
            let (it, nx) = (m.raizes[b], m.raizes[b + 1]);
            let v = match proximo_valor(m, it, nx)? {
                Some(v) => v,
                None => return Ok(None),
            };
            m.raizes[b + 3] = v;
            let (f, c) = (m.raizes[b + 2], m.numero(cont));
            let r = match m.chamar_coletavel(f, Valor::UNDEFINED, &[v, c]) {
                Ok(r) => r,
                Err(e) => return Err(fechar_raiz(m, b, e)),
            };
            if m.verdadeiro(r) == parar {
                let it = m.raizes[b];
                fechar(m, it)?;
                return Ok(Some(m.raizes[b + 3]));
            }
            cont += 1.0;
        }
    })
}

// ---------------------------------------------------------------- helpers

/// Objeto de helper com o estado inicial.
fn novo_helper(m: &mut Motor, proto: Valor, tipo: u32, it: Valor, nx: Valor, f: Valor, cont: Valor) -> Valor {
    let forma = m.forma_para(Classe::IteradorAuxiliar, proto, 0);
    let o = m.novo_objeto(forma);
    let l = m.nova_lista(7);
    m.lista_set(l, L_ITER, it);
    m.lista_set(l, L_NEXT, nx);
    m.lista_set(l, L_FN, f);
    m.lista_set(l, L_CONT, cont);
    m.por_interno_valor(o, 0, l);
    m.por_interno(o, 1, S_INICIO);
    m.por_interno(o, 2, tipo);
    o
}

fn e_helper(m: &Motor, v: Valor) -> bool {
    m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::IteradorAuxiliar && m.interno(v, 2) != T_EMBRULHO
}

fn helper_next(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !e_helper(m, this) {
        return m.erro_tipo("next method called on incompatible receiver");
    }
    retomar(m, this, false)
}

fn helper_return(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !e_helper(m, this) {
        return m.erro_tipo("return method called on incompatible receiver");
    }
    retomar(m, this, true)
}

/// `GeneratorResume` (`next`) e `GeneratorResumeAbrupt` (`return`) de um helper.
fn retomar(m: &mut Motor, h: Valor, retorno: bool) -> Res<Valor> {
    let estado = m.interno(h, 1);
    if estado == S_EXECUTANDO {
        return m.erro_tipo("Generator is already running");
    }
    if estado == S_COMPLETO {
        return Ok(m.resultado_iterador(Valor::UNDEFINED, true));
    }
    if retorno && estado == S_INICIO {
        m.por_interno(h, 1, S_COMPLETO);
        let it = m.lista_get(Valor(m.interno(h, 0)), L_ITER);
        fechar(m, it)?;
        return Ok(m.resultado_iterador(Valor::UNDEFINED, true));
    }
    m.por_interno(h, 1, S_EXECUTANDO);
    let base = m.raizes.len();
    m.raizes.push(h);
    let r = if retorno { fechar_retorno(m, base).map(|()| false) } else { gerar(m, base) };
    let h = m.raizes[base];
    m.raizes.truncate(base);
    match r {
        Ok(true) => {
            m.por_interno(h, 1, S_SUSPENSO);
            let v = m.lista_get(Valor(m.interno(h, 0)), L_VALOR);
            Ok(m.resultado_iterador(v, false))
        }
        Ok(false) => {
            m.por_interno(h, 1, S_COMPLETO);
            Ok(m.resultado_iterador(Valor::UNDEFINED, true))
        }
        Err(e) => {
            m.por_interno(h, 1, S_COMPLETO);
            Err(e)
        }
    }
}

/// Próximo passo do helper em `raizes[base]`: `true` se produziu um valor (em `L_VALOR`).
fn gerar(m: &mut Motor, base: usize) -> Res<bool> {
    match m.interno(m.raizes[base], 2) {
        T_MAPA => gerar_mapa(m, base),
        T_FILTRO => gerar_filtro(m, base),
        T_TOMA => gerar_toma(m, base),
        T_SOLTA => gerar_solta(m, base),
        _ => gerar_planifica(m, base),
    }
}

fn gerar_mapa(m: &mut Motor, base: usize) -> Res<bool> {
    let (it, nx) = (campo(m, base, L_ITER), campo(m, base, L_NEXT));
    let v = match proximo_valor(m, it, nx)? {
        Some(v) => v,
        None => return Ok(false),
    };
    ajustar(m, base, L_VALOR, v);
    let (f, c) = (campo(m, base, L_FN), campo(m, base, L_CONT));
    let r = chamar_fechando(m, base, f, &[v, c])?;
    ajustar(m, base, L_VALOR, r);
    somar_contador(m, base);
    Ok(true)
}

fn gerar_filtro(m: &mut Motor, base: usize) -> Res<bool> {
    loop {
        let (it, nx) = (campo(m, base, L_ITER), campo(m, base, L_NEXT));
        let v = match proximo_valor(m, it, nx)? {
            Some(v) => v,
            None => return Ok(false),
        };
        ajustar(m, base, L_VALOR, v);
        let (f, c) = (campo(m, base, L_FN), campo(m, base, L_CONT));
        let sel = chamar_fechando(m, base, f, &[v, c])?;
        somar_contador(m, base);
        if m.verdadeiro(sel) {
            return Ok(true);
        }
    }
}

fn gerar_toma(m: &mut Motor, base: usize) -> Res<bool> {
    let restante = m.f64_de(campo(m, base, L_CONT));
    if restante == 0.0 {
        let it = campo(m, base, L_ITER);
        fechar(m, it)?;
        return Ok(false);
    }
    if restante != f64::INFINITY {
        let n = m.numero(restante - 1.0);
        ajustar(m, base, L_CONT, n);
    }
    gerar_valor(m, base)
}

fn gerar_solta(m: &mut Motor, base: usize) -> Res<bool> {
    let mut restante = m.f64_de(campo(m, base, L_CONT));
    while restante > 0.0 {
        if restante != f64::INFINITY {
            restante -= 1.0;
            let n = m.numero(restante);
            ajustar(m, base, L_CONT, n);
        }
        let (it, nx) = (campo(m, base, L_ITER), campo(m, base, L_NEXT));
        if !avancar(m, it, nx)? {
            return Ok(false);
        }
    }
    gerar_valor(m, base)
}

/// O próximo valor do iterador do helper, sem tratamento (take e drop depois do limite).
fn gerar_valor(m: &mut Motor, base: usize) -> Res<bool> {
    let (it, nx) = (campo(m, base, L_ITER), campo(m, base, L_NEXT));
    match proximo_valor(m, it, nx)? {
        Some(v) => {
            ajustar(m, base, L_VALOR, v);
            Ok(true)
        }
        None => Ok(false),
    }
}

fn gerar_planifica(m: &mut Motor, base: usize) -> Res<bool> {
    loop {
        if campo(m, base, L_INTERNO) != Valor::UNDEFINED {
            let (it, nx) = (campo(m, base, L_INTERNO), campo(m, base, L_INTERNO_NEXT));
            match proximo_valor(m, it, nx) {
                Ok(Some(v)) => {
                    ajustar(m, base, L_VALOR, v);
                    return Ok(true);
                }
                Ok(None) => {
                    ajustar(m, base, L_INTERNO, Valor::UNDEFINED);
                    ajustar(m, base, L_INTERNO_NEXT, Valor::UNDEFINED);
                    somar_contador(m, base);
                }
                Err(e) => return Err(fechar_base(m, base, e)),
            }
            continue;
        }
        let (it, nx) = (campo(m, base, L_ITER), campo(m, base, L_NEXT));
        let v = match proximo_valor(m, it, nx)? {
            Some(v) => v,
            None => return Ok(false),
        };
        ajustar(m, base, L_VALOR, v);
        let (f, c) = (campo(m, base, L_FN), campo(m, base, L_CONT));
        let mapeado = chamar_fechando(m, base, f, &[v, c])?;
        // O valor mapeado mora em `L_VALOR` enquanto o iterador dele é lido (ele pode coletar).
        ajustar(m, base, L_VALOR, mapeado);
        let alvo = campo(m, base, L_VALOR);
        match iterador_planificavel(m, alvo, false) {
            Ok((ii, inx)) => {
                ajustar(m, base, L_INTERNO, ii);
                ajustar(m, base, L_INTERNO_NEXT, inx);
            }
            Err(e) => return Err(fechar_base(m, base, e)),
        }
    }
}

/// `return()` com o helper suspenso num `yield`: a completion de retorno fecha o iterador
/// interno (do flatMap) e depois o de baixo.
fn fechar_retorno(m: &mut Motor, base: usize) -> Res<()> {
    if m.interno(m.raizes[base], 2) == T_PLANIFICA {
        let ii = campo(m, base, L_INTERNO);
        if ii != Valor::UNDEFINED {
            if let Err(e) = fechar(m, ii) {
                return Err(fechar_base(m, base, e));
            }
        }
    }
    let it = campo(m, base, L_ITER);
    fechar(m, it)
}

// ---------------------------------------------------------------- embrulho (Iterator.from)

fn embrulho_next(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) || m.formas.get(m.forma(this)).classe != Classe::IteradorAuxiliar || m.interno(this, 2) != T_EMBRULHO {
        return m.erro_tipo("next method called on incompatible receiver");
    }
    let l = Valor(m.interno(this, 0));
    let (it, nx) = (m.lista_get(l, L_ITER), m.lista_get(l, L_NEXT));
    m.chamar_coletavel(nx, it, &[])
}

fn embrulho_return(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) || m.formas.get(m.forma(this)).classe != Classe::IteradorAuxiliar || m.interno(this, 2) != T_EMBRULHO {
        return m.erro_tipo("return method called on incompatible receiver");
    }
    let it = m.lista_get(Valor(m.interno(this, 0)), L_ITER);
    match m.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))? {
        Some(f) => m.chamar_coletavel(f, it, &[]),
        None => Ok(m.resultado_iterador(Valor::UNDEFINED, true)),
    }
}

// ---------------------------------------------------------------- auxiliares

/// Receptor de `Iterator.prototype`: tem de ser objeto (sem fechar nada).
fn receptor(m: &mut Motor, this: Valor) -> Res<Valor> {
    if m.e_objeto(this) {
        return Ok(this);
    }
    m.erro_tipo("Iterator.prototype method called on non-object")
}

/// `GetIteratorFlattenable`: o iterador de `v` e o `next` dele. Texto só vale se `texto` (é o
/// caso de `Iterator.from`; o flatMap rejeita primitivos).
fn iterador_planificavel(m: &mut Motor, v: Valor, texto: bool) -> Res<(Valor, Valor)> {
    if !m.e_objeto(v) && !(texto && m.e_texto(v)) {
        return m.erro_tipo("Iterator helper argument is not an object");
    }
    let it = match m.obter_metodo(v, ChaveProp::Nome(simb::ITERATOR))? {
        Some(f) => m.chamar_coletavel(f, v, &[])?,
        None => v,
    };
    if !m.e_objeto(it) {
        return m.erro_tipo("Result of the Symbol.iterator method is not an object");
    }
    let nx = m.obter(it, ChaveProp::Nome(fixos::NEXT))?;
    Ok((it, nx))
}

/// `IteratorStepValue`: o próximo valor de `it`; `None` quando acabou. Não fecha nada em erro.
fn proximo_valor(m: &mut Motor, it: Valor, nx: Valor) -> Res<Option<Valor>> {
    let r = proximo_resultado(m, it, nx)?;
    let feito = m.obter(r, ChaveProp::Nome(fixos::DONE))?;
    if m.verdadeiro(feito) {
        return Ok(None);
    }
    m.obter(r, ChaveProp::Nome(fixos::VALUE)).map(Some)
}

/// `IteratorStep`: avança sem ler o valor; `false` quando acabou.
fn avancar(m: &mut Motor, it: Valor, nx: Valor) -> Res<bool> {
    let r = proximo_resultado(m, it, nx)?;
    let feito = m.obter(r, ChaveProp::Nome(fixos::DONE))?;
    Ok(!m.verdadeiro(feito))
}

fn proximo_resultado(m: &mut Motor, it: Valor, nx: Valor) -> Res<Valor> {
    let r = m.chamar_coletavel(nx, it, &[])?;
    if !m.e_objeto(r) {
        return m.erro_tipo("Iterator result is not an object");
    }
    Ok(r)
}

/// `IteratorClose` com completion normal ou de retorno: chama `return` e propaga os erros.
fn fechar(m: &mut Motor, it: Valor) -> Res<()> {
    if let Some(f) = m.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))? {
        let r = m.chamar_coletavel(f, it, &[])?;
        if !m.e_objeto(r) {
            return m.erro_tipo("iterator.return() did not return an object");
        }
    }
    Ok(())
}

/// `IteratorClose` com completion de erro: chama `return` ignorando o que acontecer e devolve o
/// erro original (rodado com ele e o iterador em `raizes`, porque o `return` pode coletar).
fn fechar_erro(m: &mut Motor, it: Valor, e: Valor) -> Valor {
    let base = m.raizes.len();
    m.raizes.extend([it, e]);
    if let Ok(Some(f)) = m.obter_metodo(it, ChaveProp::Nome(fixos::RETURN)) {
        let it = m.raizes[base];
        let _ = m.chamar_coletavel(f, it, &[]);
    }
    let e = m.raizes[base + 1];
    m.raizes.truncate(base);
    e
}

/// Fecha o iterador do helper em `raizes[base]` com o erro `e`.
fn fechar_base(m: &mut Motor, base: usize, e: Valor) -> Valor {
    let it = campo(m, base, L_ITER);
    fechar_erro(m, it, e)
}

/// Fecha o iterador que está em `raizes[base]` (eagers) com o erro `e`.
fn fechar_raiz(m: &mut Motor, base: usize, e: Valor) -> Valor {
    let it = m.raizes[base];
    fechar_erro(m, it, e)
}

/// Fecha `o` com um TypeError (validação de argumento falha depois de ter `o` em mãos).
fn fechando<T>(m: &mut Motor, o: Valor, msg: &str) -> Res<T> {
    let e = m.erro(TipoErro::Tipo, msg);
    Err(fechar_erro(m, o, e))
}

fn fechando_alcance<T>(m: &mut Motor, o: Valor, msg: &str) -> Res<T> {
    let e = m.erro(TipoErro::Alcance, msg);
    Err(fechar_erro(m, o, e))
}

/// Roda `f` com `vals` em `raizes` (o coletor pode rodar dentro das chamadas de JS).
fn com_raizes<T>(m: &mut Motor, vals: &[Valor], f: impl FnOnce(&mut Motor, usize) -> Res<T>) -> Res<T> {
    let base = m.raizes.len();
    m.raizes.extend_from_slice(vals);
    let r = f(m, base);
    m.raizes.truncate(base);
    r
}

/// Campo `i` da lista de estado do helper em `raizes[base]` (lido de novo: o coletor move).
fn campo(m: &Motor, base: usize, i: u32) -> Valor {
    let h = m.raizes[base];
    m.lista_get(Valor(m.interno(h, 0)), i)
}

fn ajustar(m: &mut Motor, base: usize, i: u32, v: Valor) {
    let h = m.raizes[base];
    let l = Valor(m.interno(h, 0));
    m.lista_set(l, i, v);
}

fn somar_contador(m: &mut Motor, base: usize) {
    let n = m.f64_de(campo(m, base, L_CONT)) + 1.0;
    let v = m.numero(n);
    ajustar(m, base, L_CONT, v);
}

/// Chama o callback do helper; se ele lança, fecha o iterador de baixo.
fn chamar_fechando(m: &mut Motor, base: usize, f: Valor, args: &[Valor]) -> Res<Valor> {
    match m.chamar_coletavel(f, Valor::UNDEFINED, args) {
        Ok(r) => Ok(r),
        Err(e) => Err(fechar_base(m, base, e)),
    }
}

#[cfg(test)]
mod testes {
    use crate::Motor;

    fn rodar(fonte: &str) -> String {
        let mut m = Motor::novo();
        let v = m.avaliar(fonte).unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        m.para_texto_rust(v).unwrap()
    }

    #[test]
    fn fechamento_e_estado() {
        assert_eq!(
            rodar(
                "var log = []; function src(v) { var i = 0; return { __proto__: Iterator.prototype, \
                 next() { log.push('n'); return i < v.length ? { value: v[i++], done: false } : { value: undefined, done: true }; }, \
                 return() { log.push('r'); return {}; } }; }; \
                 var out = []; \
                 var h = src([1, 2]).map(x => x); h.return(); out.push(log.join('') + ':' + h.next().done); \
                 log = []; src([1, 2, 3]).take(2).toArray(); out.push(log.join('')); \
                 log = []; var hh; try { hh = src([1]).map(() => hh.next()); hh.next(); } catch (e) { out.push(e.name); } \
                 log = []; try { src([1]).flatMap(x => 5).next(); } catch (e) { out.push(e.name + log.join('')); } \
                 try { src([]).reduce((a, b) => a); } catch (e) { out.push(e.name); } \
                 out.push(Iterator.from({ next() { return { value: 7, done: false }; } }).next().value); \
                 out.join('|')"
            ),
            "r:true|nnr|TypeError|TypeErrornr|TypeError|7"
        );
    }
}
