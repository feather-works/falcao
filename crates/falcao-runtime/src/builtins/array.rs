//! Array: construtor, estáticos e `Array.prototype` (genéricos sobre "array-likes", com atalho
//! para array comum).

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::{ChaveProp, TEXTO_MAX};
use crate::valor::Valor;
use super::regexp::anexar;
use std::cmp::Ordering;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;
    let ap = m.intr.array_proto;
    let ctor = m.nativa("Array", 1, construtor, true);
    m.definir_propria(ctor, fixos::PROTOTYPE, ap, 0);
    m.definir_propria(ap, fixos::CONSTRUCTOR, ctor, attr::METODO);
    let c = m.chave("Array");
    m.definir_propria(g, c, ctor, attr::METODO);
    super::ligar_especie(m, ctor);
    // Array.prototype[@@unscopables] (§23.1.3.41): nomes que `with (array)` não expõe.
    let f = m.forma_para(Classe::Comum, Valor::NULL, 0);
    let u = m.novo_objeto(f);
    for n in ["at", "copyWithin", "entries", "fill", "find", "findIndex", "findLast", "findLastIndex", "flat", "flatMap", "includes", "keys", "toReversed", "toSorted", "toSpliced", "values"] {
        let k = m.chave(n);
        m.definir_propria(u, k, Valor::TRUE, attr::PADRAO);
    }
    m.definir_propria(ap, simb::UNSCOPABLES, u, attr::CONFIGURAVEL);
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

// Os laços de índice das nativas passam por estes: cada um conta uma volta (`Motor::tique`).
fn ler(m: &mut Motor, o: Valor, i: u64) -> Res<Valor> {
    m.tique()?;
    m.obter_indice(o, i)
}

fn gravar(m: &mut Motor, o: Valor, i: u64, v: Valor) -> Res<()> {
    m.tique()?;
    let k = chave_i(m, i);
    m.gravar(o, k, v, true)
}

fn criar(m: &mut Motor, o: Valor, i: u64, v: Valor) -> Res<()> {
    m.tique()?;
    let k = chave_i(m, i);
    if !m.criar_dado(o, k, v)? {
        return m.erro_tipo("Cannot add property, object is not extensible");
    }
    Ok(())
}

fn tem(m: &mut Motor, o: Valor, i: u64) -> Res<bool> {
    m.tique()?;
    if i < u64::from(u32::MAX) && m.elemento(o, i as u32).is_some() {
        return Ok(true);
    }
    let k = chave_i(m, i);
    m.tem(o, k)
}

fn apagar(m: &mut Motor, o: Valor, i: u64) -> Res<()> {
    m.tique()?;
    let k = chave_i(m, i);
    if !m.apagar(o, k)? {
        let n = i.to_string();
        return m.erro_tipo(&format!("Cannot delete property '{n}' of [object Array]"));
    }
    Ok(())
}

/// 2⁵³ − 1: o maior `length` de um objeto parecido com array (§7.1.20).
const LEN_MAX: u64 = (1 << 53) - 1;

/// TypeError se o resultado passaria de [`LEN_MAX`]: a norma lança antes de mexer em qualquer
/// item (e sem isso um `{length: 2**53-1}` prende a nativa num laço de 2⁵³ voltas).
fn cabe(m: &mut Motor, n: u64, mais: u64) -> Res<()> {
    if n.saturating_add(mais) > LEN_MAX {
        return m.erro_tipo("Array length exceeds 2^53 - 1");
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

/// Itens presentes de `o` abaixo de `n` (índice, valor), em ordem, quando o array é esparso
/// (`n` bem maior que a lista de elementos) e pular os ausentes não muda nada que o JS veja:
/// objeto comum ou array, itens de dados (na lista ou guardados por nome, nunca acessor) e
/// protótipos sem índice nenhum. O `bool` diz se todos são graváveis e configuráveis. Sem isto,
/// `[].length = 2**32-1` prende indexOf/join/reverse em 4 bilhões de voltas. None = laço geral.
fn presentes(m: &Motor, o: Valor, n: u64) -> Option<(Vec<(u64, Valor)>, bool)> {
    let el = m.elementos(o);
    let cap = if el == Valor::LISTA_VAZIA { 0 } else { m.lista_len(el) };
    if n <= 2 * u64::from(cap) + 1024 {
        return None;
    }
    let mut r: Vec<(u64, Valor)> = (0..cap).filter(|&i| u64::from(i) < n).map(|i| (u64::from(i), m.lista_get(el, i))).filter(|&(_, v)| v != Valor::BURACO).collect();
    let mut padrao = true;
    let mut p = o;
    while p != Valor::NULL {
        if !m.e_objeto(p) || !matches!(m.formas.get(m.forma(p)).classe, Classe::Comum | Classe::Array) {
            return None;
        }
        let ep = m.elementos(p);
        if p != o && ep != Valor::LISTA_VAZIA && (0..m.lista_len(ep)).any(|i| m.lista_get(ep, i) != Valor::BURACO) {
            return None;
        }
        // Índices guardados por nome: esparsos demais para a lista, com atributos próprios, ou
        // inteiros além de 2³² (chaves comuns de um array-like com `length` até 2⁵³).
        {
            for d in m.formas.descritores(m.forma(p)) {
                if d.chave.e_simbolo() {
                    continue;
                }
                let Some(i) = indice_inteiro(m.nomes.texto(d.chave)) else { continue };
                if p != o || d.attrs & attr::ACESSOR != 0 {
                    return None;
                }
                if i < n {
                    padrao &= d.attrs & (attr::GRAVAVEL | attr::CONFIGURAVEL) == attr::GRAVAVEL | attr::CONFIGURAVEL;
                    r.push((i, m.ler_campo(o, d.slot)));
                }
            }
        }
        p = m.proto_de(p);
    }
    r.sort_unstable_by_key(|e| e.0);
    Some((r, padrao))
}

/// O inteiro de uma chave que é um índice inteiro canônico ("0", "17", sem zero à esquerda).
fn indice_inteiro(t: &[u16]) -> Option<u64> {
    if t.is_empty() || t.len() > 16 || (t.len() > 1 && t[0] == u16::from(b'0')) || !t.iter().all(|&c| (0x30..=0x39).contains(&c)) {
        return None;
    }
    Some(t.iter().fold(0u64, |n, &c| n * 10 + u64::from(c - 0x30)))
}

/// Nenhum índice existe em `o` nem nos protótipos dele (objetos comuns, sem itens na lista, e
/// nenhum índice guardado como nome no motor): ler qualquer índice dá undefined sem rodar JS.
fn sem_itens(m: &Motor, o: Valor) -> bool {
    if m.indices_especiais {
        return false;
    }
    let mut x = o;
    for _ in 0..64 {
        if x == Valor::NULL {
            return true;
        }
        if !m.e_objeto(x) {
            return false;
        }
        let f = m.formas.get(m.forma(x));
        if f.classe != Classe::Comum || f.dicionario || m.elementos(x) != Valor::LISTA_VAZIA {
            return false;
        }
        x = f.proto;
    }
    false
}

fn num(m: &mut Motor, i: u64) -> Valor {
    Valor::int_se_cabe(i as i64).unwrap_or_else(|| m.numero(i as f64))
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
    Ok(Valor::bool(e_array_ou_proxy(m, m.arg(a, 0))))
}

/// IsArray: atravessa Proxy.
fn e_array_ou_proxy(m: &Motor, mut v: Valor) -> bool {
    while m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Proxy {
        v = Valor(m.interno(v, 0));
    }
    m.e_array(v)
}

/// ArraySpeciesCreate (§10.4.2.3): o array do resultado vem de `constructor[Symbol.species]`.
/// shortcut: sem o caso de outro realm (o Falcão tem um só).
fn array_da_especie(m: &mut Motor, o: Valor, n: u64) -> Res<Valor> {
    let mut c = Valor::UNDEFINED;
    if e_array_ou_proxy(m, o) {
        c = m.obter(o, ChaveProp::Nome(fixos::CONSTRUCTOR))?;
        if m.e_objeto(c) {
            c = m.obter(c, ChaveProp::Nome(simb::SPECIES))?;
            if c == Valor::NULL {
                c = Valor::UNDEFINED;
            }
        }
    }
    if c == Valor::UNDEFINED {
        if n > u64::from(u32::MAX) {
            return m.erro_alcance("Invalid array length");
        }
        let r = novo_array(m);
        m.definir_array_len(r, n as u32);
        return Ok(r);
    }
    if !m.e_construtor(c) {
        return m.erro_tipo("object.constructor[Symbol.species] is not a constructor");
    }
    let nv = num(m, n);
    m.construir(c, &[nv], c)
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
    if !iteravel.e_nulo_ou_indefinido() {
        let r = novo_array(m);
        m.percorrer_iteravel(src, |m, x, i| {
            let v = if f != Valor::UNDEFINED {
                let iv = num(m, i);
                m.chamar(f, t, &[x, iv])?
            } else {
                x
            };
            criar(m, r, i, v)
        })?;
        return Ok(r);
    }
    let itens = {
        let o = m.para_objeto(src)?;
        let n = m.comprimento_de(o)?;
        let mut v = Vec::with_capacity(n.min(1 << 20) as usize);
        if sem_itens(m, o) {
            // `Array.from({ length: n }, f)`: nenhum item em lugar nenhum, ler não roda nada.
            for _ in 0..n {
                m.tique()?;
                v.push(Valor::UNDEFINED);
            }
        } else {
            for i in 0..n {
                v.push(ler(m, o, i)?);
            }
        }
        v
    };
    let r = novo_array(m);
    // A lista já com os lugares (os itens existem: `itens` já os tem), sem esticar a cada um.
    if !itens.is_empty() && itens.len() <= u32::MAX as usize {
        let l = m.redimensionar_lista(Valor::LISTA_VAZIA, itens.len() as u32, Valor::BURACO);
        let d = r.deslocamento();
        m.g.por_valor(d, d + 8, l);
    }
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
            cabe(m, k, n)?;
            if let Some((ps, _)) = presentes(m, e, n) {
                for (i, v) in ps {
                    criar(m, r, k + i, v)?;
                }
                k += n;
                continue;
            }
            for i in 0..n {
                if tem(m, e, i)? {
                    let v = ler(m, e, i)?;
                    criar(m, r, k, v)?;
                }
                k += 1;
            }
        } else {
            cabe(m, k, 1)?;
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

/// Chama o callback para cada um dos `n` itens presentes de `o`. `para(m, extra, item, i,
/// resultado)` diz se para; devolve `extra` (que pode ter mudado de endereço). O coletor roda
/// dentro do callback: tudo o que vive entre as voltas (o objeto, o callback, o `this` dele,
/// `extra` e o item) mora em `raizes` e é relido depois de cada chamada.
fn percorrer(
    m: &mut Motor,
    o: Valor,
    n: u64,
    a: Args,
    extra: Valor,
    para: impl FnMut(&mut Motor, Valor, Valor, u64, Valor) -> Res<bool>,
) -> Res<Valor> {
    let f = funcao(m, m.arg(a, 0))?;
    let t = m.arg(a, 1);
    let base = m.raizes.len();
    m.raizes.extend([o, f, t, extra, Valor::UNDEFINED]);
    let r = percorrer_raizes(m, base, n, para).map(|()| m.raizes[base + 3]);
    m.raizes.truncate(base);
    r
}

fn percorrer_raizes(
    m: &mut Motor,
    base: usize,
    n: u64,
    para: impl FnMut(&mut Motor, Valor, Valor, u64, Valor) -> Res<bool>,
) -> Res<()> {
    let mut rep = None;
    let r = percorrer_com(m, base, n, para, &mut rep);
    m.soltar(&mut rep);
    r
}

/// O laço de [`percorrer_raizes`]. Com o callback no N2, as chamadas vão pelo repetidor
/// (`rep`): o quadro dele fica empurrado enquanto os itens vêm direto da lista, e sai antes de
/// qualquer coisa que rode JS ou lance aqui (item ausente, getter, interrupção). `para` não roda
/// JS.
fn percorrer_com(
    m: &mut Motor,
    base: usize,
    n: u64,
    mut para: impl FnMut(&mut Motor, Valor, Valor, u64, Valor) -> Res<bool>,
    rep: &mut Option<crate::interpretador::Repetidor>,
) -> Res<()> {
    for i in 0..n {
        let o = m.raizes[base];
        // Item próprio na lista (o caso comum): `tem` e `ler` de uma vez.
        let v = match (i < u64::from(u32::MAX)).then(|| m.elemento_rapido(o, i as u32)).flatten() {
            Some(v) => {
                m.voltas = m.voltas.wrapping_add(1);
                if m.voltas & 0xFFFF == 0 {
                    m.soltar(rep);
                    m.voltas = m.voltas.wrapping_sub(1);
                    m.tique()?;
                }
                v
            }
            None => {
                m.soltar(rep);
                if !tem(m, o, i)? {
                    continue;
                }
                ler(m, o, i)?
            }
        };
        m.raizes[base + 4] = v;
        let iv = num(m, i);
        let (f, t) = (m.raizes[base + 1], m.raizes[base + 2]);
        if rep.is_none() {
            *rep = m.repetidor(f, t, 3);
        }
        let r = if rep.is_some() { m.repetir(rep, &[v, iv, o])? } else { m.chamar_coletavel(f, t, &[v, iv, o])? };
        let (extra, v) = (m.raizes[base + 3], m.raizes[base + 4]);
        if para(m, extra, v, i, r)? {
            break;
        }
    }
    Ok(())
}

fn every(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut ok = true;
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    percorrer(m, o, n, a, Valor::UNDEFINED, |m, _, _, _, r| {
        ok = m.verdadeiro(r);
        Ok(!ok)
    })?;
    Ok(Valor::bool(ok))
}

fn some(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut ok = false;
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    percorrer(m, o, n, a, Valor::UNDEFINED, |m, _, _, _, r| {
        ok = m.verdadeiro(r);
        Ok(ok)
    })?;
    Ok(Valor::bool(ok))
}

fn for_each(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    percorrer(m, o, n, a, Valor::UNDEFINED, |_, _, _, _, _| Ok(false))?;
    Ok(Valor::UNDEFINED)
}

fn filter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    let r = novo_array(m);
    let mut k = 0;
    percorrer(m, o, n, a, r, |m, r, v, _, x| {
        if m.verdadeiro(x) {
            m.gravar_elemento(r, k, v);
            k += 1;
        }
        Ok(false)
    })
}

fn map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let o = m.para_objeto(this)?;
    let n = m.comprimento_de(o)?;
    if n > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
    let r = novo_array(m);
    m.definir_array_len(r, n as u32);
    // Origem densa: a lista do resultado já com os `n` lugares (buracos), sem esticar a cada
    // item (a origem já ocupa isso, então não é `length` enorme sem itens).
    if n > 0 && m.e_array(o) {
        let el = m.elementos(o);
        if el != Valor::LISTA_VAZIA && u64::from(m.lista_len(el)) >= n {
            let l = m.redimensionar_lista(Valor::LISTA_VAZIA, n as u32, Valor::BURACO);
            let d = r.deslocamento();
            m.g.por_valor(d, d + 8, l);
        }
    }
    percorrer(m, o, n, a, r, |m, r, _, i, x| {
        m.gravar_elemento(r, i as u32, x);
        Ok(false)
    })
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
    m.checar_pilha()?;
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
    let acc = if a.n >= 2 {
        m.arg(a, 1)
    } else {
        loop {
            let Some(i) = ordem.next() else { return m.erro_tipo("Reduce of empty array with no initial value") };
            if tem(m, o, i)? {
                break ler(m, o, i)?;
            }
        }
    };
    // O coletor roda no callback: o objeto, o callback e o acumulado moram em `raizes`.
    let base = m.raizes.len();
    m.raizes.extend([o, f, acc]);
    // Com o callback no N2, pelo repetidor (como em `percorrer_com`).
    let mut rep = None;
    let r = (|| {
        for i in ordem {
            let o = m.raizes[base];
            let v = match (i < u64::from(u32::MAX)).then(|| m.elemento_rapido(o, i as u32)).flatten() {
                Some(v) => {
                    m.voltas = m.voltas.wrapping_add(1);
                    if m.voltas & 0xFFFF == 0 {
                        m.soltar(&mut rep);
                        m.voltas = m.voltas.wrapping_sub(1);
                        m.tique()?;
                    }
                    v
                }
                None => {
                    m.soltar(&mut rep);
                    if !tem(m, o, i)? {
                        continue;
                    }
                    ler(m, o, i)?
                }
            };
            let iv = num(m, i);
            let (f, acc) = (m.raizes[base + 1], m.raizes[base + 2]);
            if rep.is_none() {
                rep = m.repetidor(f, Valor::UNDEFINED, 4);
            }
            m.raizes[base + 2] = if rep.is_some() { m.repetir(&mut rep, &[acc, v, iv, o])? } else { m.chamar_coletavel(f, Valor::UNDEFINED, &[acc, v, iv, o])? };
        }
        Ok(m.raizes[base + 2])
    })();
    m.soltar(&mut rep);
    m.raizes.truncate(base);
    r
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
    if let Some((ps, _)) = presentes(m, o, n) {
        // Ausente lê `undefined` (os protótipos não têm índice).
        let ps: Vec<Valor> = ps.into_iter().filter(|&(i, _)| i >= ini).map(|(_, v)| v).collect();
        if (ps.len() as u64) < n - ini && m.mesmo_valor_zero(Valor::UNDEFINED, x) {
            return Ok(Valor::TRUE);
        }
        return Ok(Valor::bool(ps.into_iter().any(|v| m.mesmo_valor_zero(v, x))));
    }
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
    if let Some((ps, _)) = presentes(m, o, n) {
        let achou = ps.into_iter().find(|&(i, v)| i >= ini && m.identicos(v, x));
        return Ok(achou.map_or(Valor::int(-1), |(i, _)| num(m, i)));
    }
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
    if let Some((ps, _)) = presentes(m, o, n) {
        let achou = ps.into_iter().rev().find(|&(i, v)| i as f64 <= ini && m.identicos(v, x));
        return Ok(achou.map_or(Valor::int(-1), |(i, _)| num(m, i)));
    }
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
    // Esparso com itens primitivos (o texto deles não roda código): o tamanho do resultado sai
    // antes, e os separadores entre os itens vão em bloco.
    if let Some((itens, _)) = presentes(m, o, n) {
        if itens.iter().all(|&(_, v)| !m.e_objeto(v) && !m.e_simbolo(v)) {
            let mut textos = Vec::with_capacity(itens.len());
            let mut total = u128::from(n.saturating_sub(1)) * sep.len() as u128;
            for (i, v) in itens {
                if !v.e_nulo_ou_indefinido() {
                    let t = m.para_texto(v)?;
                    let u = m.texto_u16_de(t);
                    total += u.len() as u128;
                    textos.push((i, u));
                }
            }
            if total > u128::from(TEXTO_MAX) {
                return m.erro_alcance("Invalid string length");
            }
            let mut r: Vec<u16> = Vec::new();
            if r.try_reserve_exact(total as usize).is_err() {
                return m.erro_alcance("Out of memory");
            }
            let mut seps = 0;
            for (i, u) in textos {
                while seps < i {
                    r.extend_from_slice(sep);
                    seps += 1;
                }
                r.extend_from_slice(&u);
            }
            while seps + 1 < n {
                r.extend_from_slice(sep);
                seps += 1;
            }
            return m.texto_u16_res(&r);
        }
    }
    m.juntando.push(o.0);
    let mut r: Vec<u16> = Vec::new();
    let res = (|| -> Res<()> {
        for i in 0..n {
            // O texto cresce no Rust, fora da gaiola: para no máximo de texto (ou na falta de
            // memória) com RangeError, sem derrubar o processo.
            if i > 0 {
                anexar(m, &mut r, sep)?;
            }
            let v = ler(m, o, i)?;
            if !v.e_nulo_ou_indefinido() {
                let t = m.para_texto(v)?;
                let u = m.texto_u16_de(t);
                anexar(m, &mut r, &u)?;
            }
        }
        Ok(())
    })();
    m.juntando.pop();
    res?;
    m.texto_u16_res(&r)
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
        return m.chamar_coletavel(j, o, &[]);
    }
    let t = super::nucleo::marca(m, o)?;
    Ok(m.texto(&format!("[object {t}]")))
}

// ---------------------------------------------------------------- pilha e fila

fn push(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    // Passando de 2³²−1 o `u32` daria a volta: o caminho lento (u64) lança o RangeError.
    if m.e_array(this)
        && m.formas.get(m.forma(this)).extensivel
        && m.propria(this, fixos::LENGTH).is_none()
        && u64::from(m.array_len(this)) + u64::from(a.n) < u64::from(u32::MAX)
    {
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
    cabe(m, n, u64::from(a.n))?;
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
    // Esparso e extensível: cada item vai de i para i-1 (o de 0 sai) e os buracos continuam.
    if m.formas.get(m.forma(o)).extensivel {
        if let Some((itens, true)) = presentes(m, o, n) {
            for &(i, _) in &itens {
                apagar(m, o, i)?;
            }
            for (i, v) in itens {
                if i > 0 {
                    gravar(m, o, i - 1, v)?;
                }
            }
            por_len(m, o, n - 1)?;
            return Ok(primeiro);
        }
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
        cabe(m, n, k)?;
        // Array esparso e extensível: cada item vai de i para i+k. Só Array: num array-like o
        // laço da norma também apaga [n, n+k), que `presentes` não lista.
        let f = m.formas.get(m.forma(o));
        // length guardado à parte = length mexido (talvez só leitura): o laço comum falha no
        // mesmo ponto que a norma.
        if f.classe == Classe::Array && f.extensivel && m.propria(o, fixos::LENGTH).is_none() {
            if let Some((itens, true)) = presentes(m, o, n) {
                for &(i, _) in &itens {
                    apagar(m, o, i)?;
                }
                for (i, v) in itens {
                    gravar(m, o, i + k, v)?;
                }
                for j in 0..a.n {
                    let v = m.arg(a, j);
                    gravar(m, o, u64::from(j), v)?;
                }
                por_len(m, o, n + k)?;
                return Ok(num(m, n + k));
            }
        }
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
    // Esparso e extensível: cada item vai de i para n-1-i e os buracos continuam buracos.
    if m.formas.get(m.forma(o)).extensivel {
        if let Some((itens, true)) = presentes(m, o, n) {
            for &(i, _) in &itens {
                apagar(m, o, i)?;
            }
            for (i, v) in itens {
                gravar(m, o, n - 1 - i, v)?;
            }
            return Ok(o);
        }
    }
    let (mut i, mut j) = (0u64, n.saturating_sub(1));
    while i < j {
        // Ordem da norma: um getter do de baixo pode encolher o array antes de olhar o de cima.
        let ti = tem(m, o, i)?;
        let vi = if ti { ler(m, o, i)? } else { Valor::UNDEFINED };
        let tj = tem(m, o, j)?;
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
    if fim.saturating_sub(ini) > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
    let r = novo_array(m);
    // Esparso: só os itens presentes (ler e testar não rodam código neles).
    if let Some((itens, _)) = presentes(m, o, n) {
        for (i, v) in itens {
            if i >= ini && i < fim {
                criar(m, r, i - ini, v)?;
            }
        }
        por_len(m, r, fim.saturating_sub(ini))?;
        return Ok(r);
    }
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
    cabe(m, n - apagar_n, novos.len() as u64)?;
    let removidos = array_da_especie(m, o, apagar_n)?;
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
        // Mesmo resultado que comparar os textos, sem criá-los (nem alocar).
        let (mut bx, mut by) = ([0u8; 11], [0u8; 11]);
        return Ok(digitos(x.como_int(), &mut bx).cmp(digitos(y.como_int(), &mut by)));
    }
    let tx = m.para_texto(x)?;
    let ty = m.para_texto(y)?;
    Ok(m.comparar_textos(tx, ty))
}

/// `i` em decimal (com `-`), escrito no fim de `buf`: o texto que `String(i)` daria, em ASCII.
fn digitos(i: i32, buf: &mut [u8; 11]) -> &[u8] {
    let mut n = i.unsigned_abs();
    let mut p = buf.len();
    loop {
        p -= 1;
        buf[p] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if i < 0 {
        p -= 1;
        buf[p] = b'-';
    }
    &buf[p..]
}

/// `f` é `(a, b) => a - b` (devolve `Some(true)`) ou `(a, b) => b - a` (`Some(false)`)?
/// Reconhecido pelo bytecode: `Carrega rA; Sub rB; Retorna` calcula `rB - rA`.
fn subtracao(m: &Motor, f: Valor) -> Option<bool> {
    use crate::funcoes::{BIT_NATIVA, F_CODIGO};
    use falcao_bytecode::Op;
    if !m.e_objeto(f) || m.formas.get(m.forma(f)).classe != Classe::Funcao {
        return None;
    }
    let c = m.interno(f, F_CODIGO);
    if c & BIT_NATIVA != 0 {
        return None;
    }
    let fb = m.protos[c as usize].codigo();
    let fl = &fb.flags;
    if fb.parametros != 2 || fl.gerador || fl.assincrona || fl.construtor_classe || fl.usa_arguments || fl.eval_direto {
        return None;
    }
    match fb.codigo.get(..6)? {
        [c, 1, s, 0, _, r] if *c == Op::Carrega as u8 && *s == Op::Sub as u8 && *r == Op::Retorna as u8 => Some(true),
        [c, 0, s, 1, _, r] if *c == Op::Carrega as u8 && *s == Op::Sub as u8 && *r == Op::Retorna as u8 => Some(false),
        _ => None,
    }
}

/// Ordenação estável por intercalação (o comparador pode lançar).
pub(super) fn ordenar(m: &mut Motor, v: &mut Vec<Valor>, f: Valor) -> Res<()> {
    if v.len() < 2 {
        return Ok(());
    }
    // Comparador `a - b` sobre números: a mesma conta, sem chamar JS (não há efeito a observar).
    if let Some(cresce) = subtracao(m, f).filter(|_| v.iter().all(|&x| m.e_numero(x))) {
        return intercalar(m, v, |m, x, y| {
            let (a, b) = (m.f64_de(x), m.f64_de(y));
            let d = if cresce { a - b } else { b - a };
            Ok(d.partial_cmp(&0.0).unwrap_or(Ordering::Equal))
        });
    }
    // Ordem padrão sobre textos e números: o texto de cada um uma vez só, não a cada comparação.
    // Ordem total: qualquer ordenação estável dá o mesmo resultado.
    if f == Valor::UNDEFINED && v.iter().all(|&x| m.e_texto(x) || m.e_numero(x)) {
        let mut chaves = Vec::with_capacity(v.len());
        for &x in v.iter() {
            let t = m.para_texto(x)?;
            chaves.push((m.texto_u16_de(t), x));
        }
        chaves.sort_by(|a, b| a.0.cmp(&b.0));
        for (d, (_, x)) in v.iter_mut().zip(chaves) {
            *d = x;
        }
        return Ok(());
    }
    intercalar(m, v, |m, x, y| comparar(m, f, x, y))
}

fn intercalar(m: &mut Motor, v: &mut Vec<Valor>, mut comparar: impl FnMut(&mut Motor, Valor, Valor) -> Res<Ordering>) -> Res<()> {
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
            while j > ini && comparar(m, v[j - 1], v[j])? == Ordering::Greater {
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
                if comparar(m, v[j], v[i])? == Ordering::Less {
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
    // Array denso: lê e grava direto na lista de elementos. O comparador pode mexer no array;
    // se mexeu (não está mais denso ou mudou de tamanho), a gravação volta ao caminho geral.
    if f == Valor::UNDEFINED || m.e_funcao(f) {
        if denso(m, o) {
            let n = m.array_len(o);
            let el = m.elementos(o);
            let mut v: Vec<Valor> = (0..n).map(|i| m.lista_get(el, i)).collect();
            ordenar(m, &mut v, f)?;
            if denso(m, o) && m.array_len(o) == n {
                let el = m.elementos(o);
                for (i, x) in v.into_iter().enumerate() {
                    m.lista_set(el, i as u32, x);
                }
            } else {
                for (i, x) in v.into_iter().enumerate() {
                    gravar(m, o, i as u64, x)?;
                }
            }
            return Ok(o);
        }
    }
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
    if n > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
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
    if n > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
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
    // ArrayCreate(newLen) antes de ler qualquer item.
    let novo = n + u64::from(a.n.max(2) - 2) - pular;
    if novo > (1 << 53) - 1 {
        return m.erro_tipo("Invalid array length");
    }
    if novo > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
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
    if n > u64::from(u32::MAX) {
        return m.erro_alcance("Invalid array length");
    }
    let mut v = Vec::new();
    for j in 0..n {
        v.push(if j == i as u64 { m.arg(a, 1) } else { ler(m, o, j)? });
    }
    Ok(m.array_de(&v))
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
    fn array_esparso_enorme_sem_laco_de_bilhoes() {
        // Sem o atalho, cada uma destas leva 4 bilhões de voltas.
        assert_eq!(
            rodar(
                "var a = []; a.length = 4294967295; a[5] = 'x'; a[70000] = NaN; \
                 var r = [a.indexOf('x'), a.lastIndexOf('x'), a.includes(NaN), a.includes(undefined), a.indexOf(1)]; \
                 try { a.join(); } catch (e) { r.push(e.name); } \
                 a.reverse(); r.push(a[4294967289], 5 in a); \
                 try { [].concat({ length: 4294967296, [Symbol.isConcatSpreadable]: true }); } catch (e) { r.push(e.name); } \
                 r.push(Array.prototype.indexOf.call({ length: 2 ** 53 - 1, [2 ** 53 - 2]: 'y' }, 'y')); \
                 r.join('|')"
            ),
            "5|5|true|true|-1|RangeError|x|false|RangeError|9007199254740990"
        );
    }

    #[test]
    fn iteravel_infinito_com_callback_que_lanca_fecha_o_iterador() {
        assert_eq!(
            rodar(
                "var fechou = 0; var it = { [Symbol.iterator]() { return { next() { return { done: false }; }, return() { fechou++; return {}; } }; } }; \
                 var r = []; \
                 try { Array.from(it, () => { throw 1; }); } catch (e) { r.push(e, fechou); } \
                 try { Object.groupBy(it, () => { throw 2; }); } catch (e) { r.push(e, fechou); } \
                 try { Map.groupBy(it, () => { throw 3; }); } catch (e) { r.push(e, fechou); } \
                 r.join('|')"
            ),
            "1|1|2|2|3|3"
        );
    }

    #[test]
    fn slice_shift_unshift_esparsos_e_criacao_grande_demais() {
        assert_eq!(
            rodar(
                "var a = []; a[4294967290] = 'u'; a[1] = 'b'; a.length = 4294967295; var r = []; \
                 var s = a.slice(4294967289); r.push(s.length, s[1], Object.keys(s).length); \
                 a.shift(); r.push(a.length, a[0], a[4294967289]); \
                 a.unshift('z'); r.push(a.length, a[0], a[1], a[4294967290]); \
                 try { Array.prototype.toReversed.call({ length: 2 ** 32 }); } catch (e) { r.push(e.name); } \
                 var o = { 0: 1, 1: 1 }; \
                 try { JSON.parse('[0,0]', function () { this[1] = o; }); } catch (e) { r.push(e.name); } \
                 r.join('|')"
            ),
            "6|u|1|4294967294|b|u|4294967295|z|b|u|RangeError|RangeError"
        );
    }
}
