//! Array: construtor, estáticos e `Array.prototype` (genéricos sobre "array-likes", com atalho
//! para array comum).

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;
use std::cmp::Ordering;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;
    let ap = m.intr.array_proto;
    let ctor = m.nativa("Array", 1, construtor, true);
    m.definir_propria(ctor, fixos::PROTOTYPE, ap, 0);
    m.definir_propria(ap, fixos::CONSTRUCTOR, ctor, attr::METODO);
    let c = m.chave("Array");
    m.definir_propria(g, c, ctor, attr::METODO);
    m.metodo(ctor, "isArray", 1, is_array);
    m.metodo(ctor, "of", 0, of);
    m.metodo(ctor, "from", 1, from);
    let metodos: &[(&str, u32, FnNativa)] = &[
        ("at", 1, at),
        ("concat", 1, concat),
        ("copyWithin", 2, copy_within),
        ("every", 1, every),
        ("fill", 1, fill),
        ("filter", 1, filter),
        ("find", 1, find),
        ("findIndex", 1, find_index),
        ("findLast", 1, find_last),
        ("findLastIndex", 1, find_last_index),
        ("flat", 0, flat),
        ("flatMap", 1, flat_map),
        ("forEach", 1, for_each),
        ("includes", 1, includes),
        ("indexOf", 1, index_of),
        ("join", 1, join),
        ("lastIndexOf", 1, last_index_of),
        ("map", 1, map),
        ("pop", 0, pop),
        ("push", 1, push),
        ("reduce", 1, reduce),
        ("reduceRight", 1, reduce_right),
        ("reverse", 0, reverse),
        ("shift", 0, shift),
        ("slice", 2, slice),
        ("some", 1, some),
        ("sort", 1, sort),
        ("splice", 2, splice),
        ("toReversed", 0, to_reversed),
        ("toSorted", 1, to_sorted),
        ("toSpliced", 2, to_spliced),
        ("unshift", 1, unshift),
        ("with", 2, with),
        ("toString", 0, to_string),
        ("toLocaleString", 0, join),
    ];
    for &(n, k, f) in metodos {
        m.metodo(ap, n, k, f);
    }
    let tag = m.texto("Array Iterator");
    let aip = m.intr.array_iterador_proto;
    m.definir_propria(aip, simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
}

// ---------------------------------------------------------------- auxiliares

fn chave_i(m: &mut Motor, i: u64) -> ChaveProp {
    if i < u64::from(u32::MAX) {
        ChaveProp::Indice(i as u32)
    } else {
        let t = m.texto(&i.to_string());
        m.chave_de_texto(t)
    }
}

fn ler(m: &mut Motor, o: Valor, i: u64) -> Res<Valor> {
    m.obter_indice(o, i)
}

fn gravar(m: &mut Motor, o: Valor, i: u64, v: Valor) -> Res<()> {
    let k = chave_i(m, i);
    m.gravar(o, k, v, true)
}

fn criar(m: &mut Motor, o: Valor, i: u64, v: Valor) -> Res<()> {
    let k = chave_i(m, i);
    if !m.criar_dado(o, k, v)? {
        return m.erro_tipo("Cannot add property, object is not extensible");
    }
    Ok(())
}

fn tem(m: &mut Motor, o: Valor, i: u64) -> Res<bool> {
    if i < u64::from(u32::MAX) && m.elemento(o, i as u32).is_some() {
        return Ok(true);
    }
    let k = chave_i(m, i);
    m.tem(o, k)
}

fn apagar(m: &mut Motor, o: Valor, i: u64) -> Res<()> {
    let k = chave_i(m, i);
    if !m.apagar(o, k)? {
        let n = i.to_string();
        return m.erro_tipo(&format!("Cannot delete property '{n}' of [object Array]"));
    }
    Ok(())
}

fn por_len(m: &mut Motor, o: Valor, n: u64) -> Res<()> {
    let v = m.numero(n as f64);
    m.gravar(o, ChaveProp::Nome(fixos::LENGTH), v, true)
}

fn funcao(m: &mut Motor, f: Valor) -> Res<Valor> {
    if !m.e_funcao(f) {
        let d = m.descrever(f);
        return m.erro_tipo(&format!("{d} is not a function"));
    }
    Ok(f)
}

/// Índice relativo (negativo conta do fim), preso em `[0, n]`.
fn relativo(m: &mut Motor, v: Valor, n: u64, padrao: u64) -> Res<u64> {
    if v == Valor::UNDEFINED {
        return Ok(padrao);
    }
    let x = m.para_inteiro(v)?;
    let n = n as f64;
    Ok(if x < 0.0 { (n + x).max(0.0) } else { x.min(n) } as u64)
}

fn novo_array(m: &mut Motor) -> Valor {
    m.array_de(&[])
}

fn num(m: &mut Motor, i: u64) -> Valor {
    m.numero(i as f64)
}

// ---------------------------------------------------------------- construtor e estáticos

fn construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let arr = if nt != Valor::UNDEFINED && nt != m.chamado {
        let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
        let p = if m.e_objeto(p) { p } else { m.intr.array_proto };
        let f = m.forma_para(Classe::Array, p, 0);
        m.novo_objeto(f)
    } else {
        novo_array(m)
    };
    if a.n == 1 {
        let v = m.arg(a, 0);
        if m.e_numero(v) {
            let x = m.f64_de(v);
            let n = crate::numeros::para_uint32(x);
            if f64::from(n) != x {
                return m.erro_alcance("Invalid array length");
            }
            m.definir_array_len(arr, n);
            return Ok(arr);
        }
    }
    for i in 0..a.n {
        let v = m.arg(a, i);
        m.gravar_elemento(arr, i, v);
    }
    Ok(arr)
}

fn is_array(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut v = m.arg(a, 0);
    while m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Proxy {
        v = Valor(m.interno(v, 0));
    }
    Ok(Valor::bool(m.e_array(v)))
}

fn of(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let itens: Vec<Valor> = (0..a.n).map(|i| m.arg(a, i)).collect();
    Ok(m.array_de(&itens))
}

fn from(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (src, f, t) = (m.arg(a, 0), m.arg(a, 1), m.arg(a, 2));
    if f != Valor::UNDEFINED {
        funcao(m, f)?;
    }
    if src.e_nulo_ou_indefinido() {
        return m.erro_tipo("Array.from requires an array-like object");
    }
    let iteravel = m.obter(src, ChaveProp::Nome(simb::ITERATOR))?;
    let itens = if !iteravel.e_nulo_ou_indefinido() {
        m.coletar_iteravel(src)?
    } else {
        let o = m.para_objeto(src)?;
        let n = m.comprimento_de(o)?;
        let mut v = Vec::with_capacity(n.min(1 << 20) as usize);
        for i in 0..n {
            v.push(ler(m, o, i)?);
        }
        v
    };
    let r = novo_array(m);
    for (i, x) in itens.into_iter().enumerate() {
        let v = if f != Valor::UNDEFINED {
            let iv = num(m, i as u64);
            m.chamar(f, t, &[x, iv])?
        } else {
            x
        };
        m.gravar_elemento(r, i as u32, v);
    }
    Ok(r)
}

// ---------------------------------------------------------------- acesso

fn at(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let x = m.para_inteiro(m.arg(a, 0))?;
    let i = if x < 0.0 { n as f64 + x } else { x };
    if i < 0.0 || i >= n as f64 {
        return Ok(Valor::UNDEFINED);
    }
    ler(m, o, i as u64)
}

fn concat(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let r = novo_array(m);
    let mut k: u64 = 0;
    let mut itens = vec![o];
    itens.extend((0..a.n).map(|i| m.arg(a, i)));
    for e in itens {
        let espalha = if m.e_objeto(e) {
            let s = m.obter(e, ChaveProp::Nome(simb::IS_CONCAT_SPREADABLE))?;
            if s != Valor::UNDEFINED {
                m.verdadeiro(s)
            } else {
                m.e_array(e)
            }
        } else {
            false
        };
        if espalha {
            let n = m.comprimento_de(e)?;
            for i in 0..n {
                if tem(m, e, i)? {
                    let v = ler(m, e, i)?;
                    criar(m, r, k, v)?;
                }
                k += 1;
            }
        } else {
            criar(m, r, k, e)?;
            k += 1;
        }
    }
    por_len(m, r, k)?;
    Ok(r)
}

fn copy_within(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let alvo = relativo(m, m.arg(a, 0), n, 0)?;
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    let fim = relativo(m, m.arg(a, 2), n, n)?;
    let qtd = (fim.saturating_sub(ini)).min(n - alvo);
    let (mut de, mut para, passo): (i64, i64, i64) = if ini < alvo && alvo < ini + qtd {
        ((ini + qtd - 1) as i64, (alvo + qtd - 1) as i64, -1)
    } else {
        (ini as i64, alvo as i64, 1)
    };
    for _ in 0..qtd {
        if tem(m, o, de as u64)? {
            let v = ler(m, o, de as u64)?;
            gravar(m, o, para as u64, v)?;
        } else {
            apagar(m, o, para as u64)?;
        }
        de += passo;
        para += passo;
    }
    Ok(o)
}

fn fill(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let v = m.arg(a, 0);
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    let fim = relativo(m, m.arg(a, 2), n, n)?;
    for i in ini..fim {
        gravar(m, o, i, v)?;
    }
    Ok(o)
}

// ---------------------------------------------------------------- iteração com callback

/// Percorre os itens existentes chamando `f(v, i, o)`; `para` decide se para (e com o quê).
fn percorrer(m: &mut Motor, this: Valor, a: Args, mut para: impl FnMut(&mut Motor, Valor, u64, Valor) -> Res<bool>) -> Res<()> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let f = funcao(m, m.arg(a, 0))?;
    let t = m.arg(a, 1);
    for i in 0..n {
        if !tem(m, o, i)? {
            continue;
        }
        let v = ler(m, o, i)?;
        let iv = num(m, i);
        let r = m.chamar(f, t, &[v, iv, o])?;
        if para(m, v, i, r)? {
            break;
        }
    }
    Ok(())
}

fn every(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut ok = true;
    percorrer(m, this, a, |m, _, _, r| {
        ok = m.verdadeiro(r);
        Ok(!ok)
    })?;
    Ok(Valor::bool(ok))
}

fn some(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut ok = false;
    percorrer(m, this, a, |m, _, _, r| {
        ok = m.verdadeiro(r);
        Ok(ok)
    })?;
    Ok(Valor::bool(ok))
}

fn for_each(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    percorrer(m, this, a, |_, _, _, _| Ok(false))?;
    Ok(Valor::UNDEFINED)
}

fn filter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let r = novo_array(m);
    let mut k = 0;
    percorrer(m, this, a, |m, v, _, x| {
        if m.verdadeiro(x) {
            m.gravar_elemento(r, k, v);
            k += 1;
        }
        Ok(false)
    })?;
    Ok(r)
}

fn map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    if n > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
    let r = novo_array(m);
    m.definir_array_len(r, n as u32);
    percorrer(m, o, a, |m, _, i, x| {
        m.gravar_elemento(r, i as u32, x);
        Ok(false)
    })?;
    Ok(r)
}

/// find/findIndex/findLast/findLastIndex (visitam também os buracos).
fn achar(m: &mut Motor, this: Valor, a: Args, do_fim: bool, indice: bool) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let f = funcao(m, m.arg(a, 0))?;
    let t = m.arg(a, 1);
    for j in 0..n {
        let i = if do_fim { n - 1 - j } else { j };
        let v = ler(m, o, i)?;
        let iv = num(m, i);
        let r = m.chamar(f, t, &[v, iv, o])?;
        if m.verdadeiro(r) {
            return Ok(if indice { iv } else { v });
        }
    }
    Ok(if indice { Valor::int(-1) } else { Valor::UNDEFINED })
}

fn find(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    achar(m, this, a, false, false)
}

fn find_index(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    achar(m, this, a, false, true)
}

fn find_last(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    achar(m, this, a, true, false)
}

fn find_last_index(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    achar(m, this, a, true, true)
}

fn achatar_em(m: &mut Motor, r: Valor, k: &mut u64, src: Valor, fundo: f64, mapa: Option<(Valor, Valor)>) -> Res<()> {
    let n = m.comprimento_de(src)?;
    for i in 0..n {
        if !tem(m, src, i)? {
            continue;
        }
        let mut v = ler(m, src, i)?;
        if let Some((f, t)) = mapa {
            let iv = num(m, i);
            v = m.chamar(f, t, &[v, iv, src])?;
        }
        if fundo > 0.0 && m.e_array(v) {
            achatar_em(m, r, k, v, fundo - 1.0, None)?;
        } else {
            criar(m, r, *k, v)?;
            *k += 1;
        }
    }
    Ok(())
}

fn flat(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let d = m.arg(a, 0);
    let fundo = if d == Valor::UNDEFINED { 1.0 } else { m.para_inteiro(d)?.max(0.0) };
    let r = novo_array(m);
    let mut k = 0;
    achatar_em(m, r, &mut k, o, fundo, None)?;
    Ok(r)
}

fn flat_map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let f = funcao(m, m.arg(a, 0))?;
    let t = m.arg(a, 1);
    let r = novo_array(m);
    let mut k = 0;
    achatar_em(m, r, &mut k, o, 1.0, Some((f, t)))?;
    Ok(r)
}

fn reduzir(m: &mut Motor, this: Valor, a: Args, do_fim: bool) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let f = funcao(m, m.arg(a, 0))?;
    let ordem: Box<dyn Iterator<Item = u64>> = if do_fim { Box::new((0..n).rev()) } else { Box::new(0..n) };
    let mut ordem = ordem.peekable();
    let mut acc = if a.n >= 2 {
        m.arg(a, 1)
    } else {
        loop {
            let Some(i) = ordem.next() else { return m.erro_tipo("Reduce of empty array with no initial value") };
            if tem(m, o, i)? {
                break ler(m, o, i)?;
            }
        }
    };
    for i in ordem {
        if !tem(m, o, i)? {
            continue;
        }
        let v = ler(m, o, i)?;
        let iv = num(m, i);
        acc = m.chamar(f, Valor::UNDEFINED, &[acc, v, iv, o])?;
    }
    Ok(acc)
}

fn reduce(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    reduzir(m, this, a, false)
}

fn reduce_right(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    reduzir(m, this, a, true)
}

// ---------------------------------------------------------------- busca

fn includes(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let x = m.arg(a, 0);
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    for i in ini..n {
        let v = ler(m, o, i)?;
        if m.mesmo_valor_zero(v, x) {
            return Ok(Valor::TRUE);
        }
    }
    Ok(Valor::FALSE)
}

fn index_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let x = m.arg(a, 0);
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    for i in ini..n {
        if tem(m, o, i)? {
            let v = ler(m, o, i)?;
            if m.identicos(v, x) {
                return Ok(num(m, i));
            }
        }
    }
    Ok(Valor::int(-1))
}

fn last_index_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    if n == 0 {
        return Ok(Valor::int(-1));
    }
    let x = m.arg(a, 0);
    let ini = if a.n > 1 {
        let k = m.para_inteiro(m.arg(a, 1))?;
        if k < 0.0 {
            n as f64 + k
        } else {
            k.min(n as f64 - 1.0)
        }
    } else {
        n as f64 - 1.0
    };
    let mut i = ini;
    while i >= 0.0 {
        let iu = i as u64;
        if tem(m, o, iu)? {
            let v = ler(m, o, iu)?;
            if m.identicos(v, x) {
                return Ok(num(m, iu));
            }
        }
        i -= 1.0;
    }
    Ok(Valor::int(-1))
}

// ---------------------------------------------------------------- textos

fn juntar(m: &mut Motor, o: Valor, sep: &[u16]) -> Res<Valor> {
    let n = m.comprimento_de(o)?;
    // Ciclo (a.join dentro de a): devolve vazio como os navegadores.
    if m.juntando.contains(&o.0) {
        return Ok(Valor::VAZIA);
    }
    m.juntando.push(o.0);
    let mut r: Vec<u16> = Vec::new();
    let res = (|| -> Res<()> {
        for i in 0..n {
            if i > 0 {
                r.extend_from_slice(sep);
            }
            let v = ler(m, o, i)?;
            if !v.e_nulo_ou_indefinido() {
                let t = m.para_texto(v)?;
                r.extend(m.texto_u16_de(t));
            }
        }
        Ok(())
    })();
    m.juntando.pop();
    res?;
    Ok(m.texto_u16(&r))
}

fn join(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let s = m.arg(a, 0);
    let sep = if s == Valor::UNDEFINED {
        vec![b',' as u16]
    } else {
        let t = m.para_texto(s)?;
        m.texto_u16_de(t)
    };
    juntar(m, o, &sep)
}

fn to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let kj = m.chave("join");
    let j = m.obter(o, ChaveProp::Nome(kj))?;
    if m.e_funcao(j) {
        return m.chamar(j, o, &[]);
    }
    let t = super::nucleo::marca(m, o)?;
    Ok(m.texto(&format!("[object {t}]")))
}

// ---------------------------------------------------------------- pilha e fila

fn push(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if m.e_array(this) && m.formas.get(m.forma(this)).extensivel && m.propria(this, fixos::LENGTH).is_none() {
        let mut n = m.array_len(this);
        for i in 0..a.n {
            let v = m.arg(a, i);
            m.gravar_elemento(this, n, v);
            n += 1;
        }
        return Ok(num(m, u64::from(n)));
    }
    let o = m.para_objeto(this)?;
    let mut n = m.comprimento_de(o)?;
    for i in 0..a.n {
        let v = m.arg(a, i);
        gravar(m, o, n, v)?;
        n += 1;
    }
    por_len(m, o, n)?;
    Ok(num(m, n))
}

fn pop(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    if n == 0 {
        por_len(m, o, 0)?;
        return Ok(Valor::UNDEFINED);
    }
    let v = ler(m, o, n - 1)?;
    apagar(m, o, n - 1)?;
    por_len(m, o, n - 1)?;
    Ok(v)
}

fn shift(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    if n == 0 {
        por_len(m, o, 0)?;
        return Ok(Valor::UNDEFINED);
    }
    let primeiro = ler(m, o, 0)?;
    if denso(m, o) {
        let el = m.elementos(o);
        let itens: Vec<Valor> = (1..n as u32).map(|i| m.lista_get(el, i)).collect();
        for (i, v) in itens.into_iter().enumerate() {
            m.lista_set(el, i as u32, v);
        }
        m.definir_array_len(o, n as u32 - 1);
        return Ok(primeiro);
    }
    for i in 1..n {
        if tem(m, o, i)? {
            let v = ler(m, o, i)?;
            gravar(m, o, i - 1, v)?;
        } else {
            apagar(m, o, i - 1)?;
        }
    }
    apagar(m, o, n - 1)?;
    por_len(m, o, n - 1)?;
    Ok(primeiro)
}

/// Array comum sem buracos, extensível e com os itens todos na lista de elementos.
fn denso(m: &mut Motor, o: Valor) -> bool {
    if !m.e_array(o) || m.propria(o, fixos::LENGTH).is_some() || !m.formas.get(m.forma(o)).extensivel || m.indices_especiais {
        return false;
    }
    let n = m.array_len(o);
    let el = m.elementos(o);
    if n == 0 {
        return true;
    }
    if el == Valor::LISTA_VAZIA || m.lista_len(el) < n {
        return false;
    }
    !m.formas.get(m.forma(o)).elementos.com_buracos() || (0..n).all(|i| m.lista_get(el, i) != Valor::BURACO)
}

fn unshift(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let k = u64::from(a.n);
    if k > 0 {
        let mut i = n;
        while i > 0 {
            i -= 1;
            if tem(m, o, i)? {
                let v = ler(m, o, i)?;
                gravar(m, o, i + k, v)?;
            } else {
                apagar(m, o, i + k)?;
            }
        }
        for j in 0..a.n {
            let v = m.arg(a, j);
            gravar(m, o, u64::from(j), v)?;
        }
    }
    por_len(m, o, n + k)?;
    Ok(num(m, n + k))
}

fn reverse(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let (mut i, mut j) = (0u64, n.saturating_sub(1));
    while i < j {
        let (ti, tj) = (tem(m, o, i)?, tem(m, o, j)?);
        let vi = if ti { ler(m, o, i)? } else { Valor::UNDEFINED };
        let vj = if tj { ler(m, o, j)? } else { Valor::UNDEFINED };
        if tj {
            gravar(m, o, i, vj)?;
        } else {
            apagar(m, o, i)?;
        }
        if ti {
            gravar(m, o, j, vi)?;
        } else {
            apagar(m, o, j)?;
        }
        i += 1;
        j -= 1;
    }
    Ok(o)
}

fn slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fim = relativo(m, m.arg(a, 1), n, n)?;
    let r = novo_array(m);
    let mut k = 0;
    for i in ini..fim.max(ini) {
        if tem(m, o, i)? {
            let v = ler(m, o, i)?;
            criar(m, r, k, v)?;
        }
        k += 1;
    }
    por_len(m, r, k)?;
    Ok(r)
}

fn splice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let apagar_n = if a.n == 0 {
        0
    } else if a.n == 1 {
        n - ini
    } else {
        let x = m.para_inteiro(m.arg(a, 1))?;
        (x.max(0.0) as u64).min(n - ini)
    };
    let novos: Vec<Valor> = (2..a.n.max(2)).map(|i| m.arg(a, i)).collect();
    let removidos = novo_array(m);
    for j in 0..apagar_n {
        if tem(m, o, ini + j)? {
            let v = ler(m, o, ini + j)?;
            criar(m, removidos, j, v)?;
        }
    }
    por_len(m, removidos, apagar_n)?;
    let k = novos.len() as u64;
    if k < apagar_n {
        for i in ini..(n - apagar_n) {
            let (de, para) = (i + apagar_n, i + k);
            if tem(m, o, de)? {
                let v = ler(m, o, de)?;
                gravar(m, o, para, v)?;
            } else {
                apagar(m, o, para)?;
            }
        }
        let mut i = n;
        while i > n - apagar_n + k {
            i -= 1;
            apagar(m, o, i)?;
        }
    } else if k > apagar_n {
        let mut i = n - apagar_n;
        while i > ini {
            i -= 1;
            let (de, para) = (i + apagar_n, i + k);
            if tem(m, o, de)? {
                let v = ler(m, o, de)?;
                gravar(m, o, para, v)?;
            } else {
                apagar(m, o, para)?;
            }
        }
    }
    for (j, v) in novos.into_iter().enumerate() {
        gravar(m, o, ini + j as u64, v)?;
    }
    por_len(m, o, n - apagar_n + k)?;
    Ok(removidos)
}

// ---------------------------------------------------------------- ordenar

fn comparar(m: &mut Motor, f: Valor, x: Valor, y: Valor) -> Res<Ordering> {
    if x == Valor::UNDEFINED || y == Valor::UNDEFINED {
        return Ok(match (x == Valor::UNDEFINED, y == Valor::UNDEFINED) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            _ => Ordering::Less,
        });
    }
    if f != Valor::UNDEFINED {
        let r = m.chamar(f, Valor::UNDEFINED, &[x, y])?;
        let v = m.para_numero(r)?;
        return Ok(if v < 0.0 {
            Ordering::Less
        } else if v > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        });
    }
    if x.e_int() && y.e_int() {
        // Mesmo resultado que comparar os textos, sem criá-los.
        let (a, b) = (x.como_int(), y.como_int());
        if a >= 0 && b >= 0 {
            return Ok(a.to_string().cmp(&b.to_string()));
        }
    }
    let tx = m.para_texto(x)?;
    let ty = m.para_texto(y)?;
    Ok(m.comparar_textos(tx, ty))
}

/// Ordenação estável por intercalação (o comparador pode lançar).
pub(super) fn ordenar(m: &mut Motor, v: &mut Vec<Valor>, f: Valor) -> Res<()> {
    let n = v.len();
    if n < 2 {
        return Ok(());
    }
    let mut aux = v.clone();
    let mut largura = 1;
    // Inserção em blocos de 8 primeiro.
    const BLOCO: usize = 8;
    for ini in (0..n).step_by(BLOCO) {
        let fim = (ini + BLOCO).min(n);
        for i in ini + 1..fim {
            let mut j = i;
            while j > ini && comparar(m, f, v[j - 1], v[j])? == Ordering::Greater {
                v.swap(j - 1, j);
                j -= 1;
            }
        }
    }
    largura = largura.max(BLOCO);
    while largura < n {
        let mut ini = 0;
        while ini < n {
            let meio = (ini + largura).min(n);
            let fim = (ini + 2 * largura).min(n);
            let (mut i, mut j, mut k) = (ini, meio, ini);
            while i < meio && j < fim {
                if comparar(m, f, v[j], v[i])? == Ordering::Less {
                    aux[k] = v[j];
                    j += 1;
                } else {
                    aux[k] = v[i];
                    i += 1;
                }
                k += 1;
            }
            aux[k..k + meio - i].copy_from_slice(&v[i..meio]);
            k += meio - i;
            aux[k..k + fim - j].copy_from_slice(&v[j..fim]);
            ini += 2 * largura;
        }
        std::mem::swap(v, &mut aux);
        largura *= 2;
    }
    Ok(())
}

fn itens_para_ordenar(m: &mut Motor, o: Valor, f: Valor) -> Res<(Vec<Valor>, u64)> {
    if f != Valor::UNDEFINED {
        funcao(m, f)?;
    }
    let n = m.comprimento_de(o)?;
    let mut v = Vec::new();
    for i in 0..n {
        if tem(m, o, i)? {
            v.push(ler(m, o, i)?);
        }
    }
    ordenar(m, &mut v, f)?;
    Ok((v, n))
}

fn sort(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    let o = m.para_objeto(this)?;
    let (v, n) = itens_para_ordenar(m, o, f)?;
    let k = v.len() as u64;
    for (i, x) in v.into_iter().enumerate() {
        gravar(m, o, i as u64, x)?;
    }
    for i in k..n {
        apagar(m, o, i)?;
    }
    Ok(o)
}

fn to_sorted(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    let o = m.para_objeto(this)?;
    if f != Valor::UNDEFINED {
        funcao(m, f)?;
    }
    let n = m.comprimento_de(o)?;
    let mut v = Vec::new();
    for i in 0..n {
        v.push(ler(m, o, i)?);
    }
    ordenar(m, &mut v, f)?;
    Ok(m.array_de(&v))
}

fn to_reversed(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let mut v = Vec::new();
    for i in (0..n).rev() {
        v.push(ler(m, o, i)?);
    }
    Ok(m.array_de(&v))
}

fn to_spliced(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let pular = if a.n == 0 {
        0
    } else if a.n == 1 {
        n - ini
    } else {
        let x = m.para_inteiro(m.arg(a, 1))?;
        (x.max(0.0) as u64).min(n - ini)
    };
    let mut v = Vec::new();
    for i in 0..ini {
        v.push(ler(m, o, i)?);
    }
    for i in 2..a.n.max(2) {
        v.push(m.arg(a, i));
    }
    for i in ini + pular..n {
        v.push(ler(m, o, i)?);
    }
    Ok(m.array_de(&v))
}

fn with(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let x = m.para_inteiro(m.arg(a, 0))?;
    let i = if x < 0.0 { n as f64 + x } else { x };
    if i < 0.0 || i >= n as f64 {
        return m.erro_alcance("Invalid index");
    }
    let mut v = Vec::new();
    for j in 0..n {
        v.push(if j == i as u64 { m.arg(a, 1) } else { ler(m, o, j)? });
    }
    Ok(m.array_de(&v))
}
