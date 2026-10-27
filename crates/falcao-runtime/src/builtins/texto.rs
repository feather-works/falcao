//! `String` (§22.1): construtor, `String.fromCharCode`, `fromCodePoint` e `raw`, e os métodos
//! de `String.prototype`.
//!
//! Os métodos leem o texto de `this` (primitivo ou caixa). Quando o argumento tem o método do
//! símbolo correspondente (`split`, `replace`, `replaceAll`, `match`, `matchAll`, `search`),
//! esse método é chamado; aqui fica só o caso texto.

use crate::formas::{attr, Chave, Classe};
use crate::funcoes::{Args, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::{ChaveProp, TEXTO_MAX};
use crate::valor::Valor;
use std::cmp::Ordering;

pub(super) fn instalar(m: &mut Motor) {
    let proto = m.intr.texto_proto;
    let ctor = m.nativa("String", 1, construtor, true);
    m.definir_propria(ctor, fixos::PROTOTYPE, proto, 0);
    m.definir_propria(proto, fixos::CONSTRUCTOR, ctor, attr::METODO);
    m.metodo(ctor, "fromCharCode", 1, from_char_code);
    m.metodo(ctor, "fromCodePoint", 1, from_code_point);
    m.metodo(ctor, "raw", 1, raw);
    let g = m.intr.global;
    let c = m.chave("String");
    m.definir_propria(g, c, ctor, attr::METODO);

    m.metodo(proto, "at", 1, at);
    m.metodo(proto, "charAt", 1, char_at);
    m.metodo(proto, "charCodeAt", 1, char_code_at);
    m.metodo(proto, "codePointAt", 1, code_point_at);
    m.metodo(proto, "concat", 1, concat);
    m.metodo(proto, "endsWith", 1, ends_with);
    m.metodo(proto, "includes", 1, includes);
    m.metodo(proto, "indexOf", 1, index_of);
    m.metodo(proto, "lastIndexOf", 1, last_index_of);
    m.metodo(proto, "localeCompare", 1, locale_compare);
    m.metodo(proto, "normalize", 0, normalize);
    m.metodo(proto, "padEnd", 1, pad_end);
    m.metodo(proto, "padStart", 1, pad_start);
    m.metodo(proto, "repeat", 1, repeat);
    m.metodo(proto, "slice", 2, slice);
    m.metodo(proto, "startsWith", 1, starts_with);
    m.metodo(proto, "substring", 2, substring);
    m.metodo(proto, "substr", 2, substr);
    m.metodo(proto, "toLowerCase", 0, to_lower);
    m.metodo(proto, "toUpperCase", 0, to_upper);
    m.metodo(proto, "toLocaleLowerCase", 0, to_lower);
    m.metodo(proto, "toLocaleUpperCase", 0, to_upper);
    m.metodo(proto, "trim", 0, trim);
    m.metodo(proto, "trimStart", 0, trim_start);
    m.metodo(proto, "trimEnd", 0, trim_end);
    m.metodo(proto, "toString", 0, to_string);
    m.metodo(proto, "valueOf", 0, value_of);
    m.metodo(proto, "isWellFormed", 0, is_well_formed);
    m.metodo(proto, "toWellFormed", 0, to_well_formed);
    m.metodo(proto, "split", 2, split);
    m.metodo(proto, "replace", 2, replace);
    m.metodo(proto, "replaceAll", 2, replace_all);
    m.metodo(proto, "match", 1, regex_match);
    m.metodo(proto, "matchAll", 1, regex_match_all);
    m.metodo(proto, "search", 1, regex_search);
    let it = m.nativa("[Symbol.iterator]", 0, iterador, false);
    m.definir_propria(proto, simb::ITERATOR, it, attr::METODO);
}

// ---------------------------------------------------------------- auxiliares

/// Valor interno de um objeto caixa (`None` se `v` não é caixa).
fn interno_de_caixa(m: &Motor, v: Valor) -> Option<Valor> {
    if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Caixa {
        Some(Valor(m.interno(v, 0)))
    } else {
        None
    }
}

fn e_regex(m: &Motor, v: Valor) -> bool {
    m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Regex
}

/// `this` de um método de String: o texto (primitivo ou caixa). `null`/`undefined` lançam.
fn texto_this(m: &mut Motor, this: Valor, nome: &str) -> Res<Valor> {
    if m.e_texto(this) {
        return Ok(this);
    }
    if this.e_nulo_ou_indefinido() {
        return m.erro_tipo(&format!("String.prototype.{nome} called on null or undefined"));
    }
    if let Some(p) = interno_de_caixa(m, this) {
        if m.e_texto(p) {
            return Ok(p);
        }
    }
    m.para_texto(this)
}

/// `thisStringValue`: só aceita texto ou caixa de texto.
fn texto_do_valor(m: &mut Motor, this: Valor, nome: &str) -> Res<Valor> {
    if m.e_texto(this) {
        return Ok(this);
    }
    if let Some(p) = interno_de_caixa(m, this) {
        if m.e_texto(p) {
            return Ok(p);
        }
    }
    m.erro_tipo(&format!("String.prototype.{nome} requires that 'this' be a String"))
}

/// Índice de `s` em `t` a partir de `de` (comparação por unidades UTF-16).
fn procurar(t: &[u16], s: &[u16], de: usize) -> Option<usize> {
    if s.is_empty() {
        return (de <= t.len()).then_some(de);
    }
    if s.len() > t.len() {
        return None;
    }
    (de..=t.len() - s.len()).find(|&i| t[i..i + s.len()] == *s)
}

/// `s` aparece em `t` exatamente na posição `i`?
fn casa_em(t: &[u16], s: &[u16], i: usize) -> bool {
    i + s.len() <= t.len() && t[i..i + s.len()] == *s
}

/// Posição relativa (negativa conta do fim), limitada a [0, n].
fn relativo(x: f64, n: f64) -> f64 {
    if x < 0.0 {
        (n + x).max(0.0)
    } else {
        x.min(n)
    }
}

/// Limita `x` a [0, n].
fn limite(x: f64, n: f64) -> f64 {
    x.max(0.0).min(n)
}

/// Espaços do `trim` do JS (WhiteSpace e LineTerminator).
fn e_espaco(c: u16) -> bool {
    matches!(
        c,
        0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF
    )
}

/// Método do símbolo `s` de `alvo` (`undefined` se `alvo` é nulo ou não tem o método).
fn metodo_do_simbolo(m: &mut Motor, alvo: Valor, s: Chave) -> Res<Valor> {
    if alvo.e_nulo_ou_indefinido() {
        return Ok(Valor::UNDEFINED);
    }
    m.obter(alvo, ChaveProp::Nome(s))
}

/// `Symbol(desc)` (SymbolDescriptiveString).
fn descritiva(m: &mut Motor, s: Valor) -> Valor {
    let d = m.descricao_do_simbolo(s);
    let ini = m.texto("Symbol(");
    let meio = if d == Valor::UNDEFINED { Valor::VAZIA } else { d };
    let x = m.concatenar(ini, meio);
    let fim = m.texto(")");
    m.concatenar(x, fim)
}

/// Argumento de `startsWith`, `endsWith` e `includes`: regex não vale, o resto vira texto.
fn texto_de_busca(m: &mut Motor, v: Valor, nome: &str) -> Res<Valor> {
    if e_regex(m, v) {
        return m.erro_tipo(&format!("First argument to String.prototype.{nome} must not be a regular expression"));
    }
    m.para_texto(v)
}

/// Expande `$$`, `$&`, `` $` `` e `$'` do texto de substituição. Sem grupos de captura no
/// caso texto, `$n` e `$<nome>` ficam literais.
fn substituicao(modelo: &[u16], t: &[u16], pos: usize, casado: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(modelo.len());
    let mut i = 0;
    while i < modelo.len() {
        if modelo[i] == u16::from(b'$') && i + 1 < modelo.len() {
            let prox = modelo[i + 1];
            if prox == u16::from(b'$') {
                out.push(prox);
                i += 2;
                continue;
            }
            if prox == u16::from(b'&') {
                out.extend_from_slice(casado);
                i += 2;
                continue;
            }
            if prox == u16::from(b'`') {
                out.extend_from_slice(&t[..pos]);
                i += 2;
                continue;
            }
            if prox == u16::from(b'\'') {
                out.extend_from_slice(&t[pos + casado.len()..]);
                i += 2;
                continue;
            }
        }
        out.push(modelo[i]);
        i += 1;
    }
    out
}

/// Corrige surrogates soltos (troca por U+FFFD). Devolve também se houve troca.
fn corrigir_surrogates(u: &[u16]) -> (Vec<u16>, bool) {
    let mut out = Vec::with_capacity(u.len());
    let mut mudou = false;
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if (0xD800..0xDC00).contains(&c) && i + 1 < u.len() && (0xDC00..0xE000).contains(&u[i + 1]) {
            out.push(c);
            out.push(u[i + 1]);
            i += 2;
            continue;
        }
        if (0xD800..0xE000).contains(&c) {
            out.push(0xFFFD);
            mudou = true;
        } else {
            out.push(c);
        }
        i += 1;
    }
    (out, mudou)
}

// ---------------------------------------------------------------- construtor e estáticos

fn construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let s = if a.n == 0 {
        Valor::VAZIA
    } else {
        let v = m.arg(a, 0);
        if nt == Valor::UNDEFINED && m.e_simbolo(v) {
            descritiva(m, v)
        } else {
            m.para_texto(v)?
        }
    };
    if nt == Valor::UNDEFINED {
        return Ok(s);
    }
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = if m.e_objeto(p) { p } else { m.intr.texto_proto };
    let f = m.forma_para(Classe::Caixa, p, 0);
    let o = m.novo_objeto(f);
    m.por_interno_valor(o, 0, s);
    Ok(o)
}

fn from_char_code(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut u: Vec<u16> = Vec::with_capacity(a.n as usize);
    for i in 0..a.n {
        let v = m.arg(a, i);
        u.push((m.para_uint32(v)? & 0xFFFF) as u16);
    }
    Ok(m.texto_u16(&u))
}

fn from_code_point(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut u: Vec<u16> = Vec::with_capacity(a.n as usize);
    for i in 0..a.n {
        let v = m.arg(a, i);
        let x = m.para_numero(v)?;
        if x.fract() != 0.0 || !(0.0..=1_114_111.0).contains(&x) {
            let s = crate::numeros::numero_para_texto(x);
            return m.erro_alcance(&format!("Invalid code point {s}"));
        }
        let cp = x as u32;
        if cp < 0x10000 {
            u.push(cp as u16);
        } else {
            let c = cp - 0x10000;
            u.push(0xD800 + (c >> 10) as u16);
            u.push(0xDC00 + (c & 0x3FF) as u16);
        }
    }
    Ok(m.texto_u16(&u))
}

fn raw(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let modelo = m.arg(a, 0);
    let cooked = m.para_objeto(modelo)?;
    let r = m.obter(cooked, ChaveProp::Nome(fixos::RAW))?;
    let bruto = m.para_objeto(r)?;
    let n = m.comprimento_de(bruto)?;
    let mut acc = Valor::VAZIA;
    for i in 0..n {
        let seg = m.obter_indice(bruto, i)?;
        let seg = m.para_texto(seg)?;
        acc = m.concatenar(acc, seg);
        if i + 1 == n {
            break;
        }
        // Substituição i (argumento i + 1), quando existe.
        if i + 1 < u64::from(a.n) {
            let sub = m.arg(a, (i + 1) as u32);
            let sub = m.para_texto(sub)?;
            acc = m.concatenar(acc, sub);
        }
    }
    Ok(acc)
}

// ---------------------------------------------------------------- acesso e busca

fn at(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "at")?;
    let n = f64::from(m.texto_len(t));
    let v = m.arg(a, 0);
    let mut k = m.para_inteiro(v)?;
    if k < 0.0 {
        k += n;
    }
    if k < 0.0 || k >= n {
        return Ok(Valor::UNDEFINED);
    }
    let i = k as u32;
    Ok(m.subtexto(t, i, i + 1))
}

fn char_at(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "charAt")?;
    let v = m.arg(a, 0);
    let k = m.para_inteiro(v)?;
    if k < 0.0 || k >= f64::from(m.texto_len(t)) {
        return Ok(Valor::VAZIA);
    }
    let i = k as u32;
    Ok(m.subtexto(t, i, i + 1))
}

fn char_code_at(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "charCodeAt")?;
    let v = m.arg(a, 0);
    let k = m.para_inteiro(v)?;
    if k < 0.0 || k >= f64::from(m.texto_len(t)) {
        return Ok(m.numero(f64::NAN));
    }
    let c = m.unidade_em(t, k as u32);
    Ok(m.numero(f64::from(c)))
}

fn code_point_at(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "codePointAt")?;
    let v = m.arg(a, 0);
    let k = m.para_inteiro(v)?;
    let n = m.texto_len(t);
    if k < 0.0 || k >= f64::from(n) {
        return Ok(Valor::UNDEFINED);
    }
    let i = k as u32;
    let c = m.unidade_em(t, i);
    let mut cp = u32::from(c);
    if (0xD800..0xDC00).contains(&c) && i + 1 < n {
        let d = m.unidade_em(t, i + 1);
        if (0xDC00..0xE000).contains(&d) {
            cp = (u32::from(c) - 0xD800) * 0x400 + (u32::from(d) - 0xDC00) + 0x10000;
        }
    }
    Ok(m.numero(f64::from(cp)))
}

fn concat(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut acc = texto_this(m, this, "concat")?;
    for i in 0..a.n {
        let v = m.arg(a, i);
        let s = m.para_texto(v)?;
        acc = m.concatenar(acc, s);
    }
    Ok(acc)
}

fn starts_with(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "startsWith")?;
    let v0 = m.arg(a, 0);
    let s = texto_de_busca(m, v0, "startsWith")?;
    let v1 = m.arg(a, 1);
    let p = m.para_inteiro(v1)?;
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    let ini = limite(p, tu.len() as f64) as usize;
    Ok(Valor::bool(casa_em(&tu, &su, ini)))
}

fn ends_with(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "endsWith")?;
    let v0 = m.arg(a, 0);
    let s = texto_de_busca(m, v0, "endsWith")?;
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    let n = tu.len() as f64;
    let v1 = m.arg(a, 1);
    let fim_f = if v1 == Valor::UNDEFINED {
        n
    } else {
        let p = m.para_inteiro(v1)?;
        limite(p, n)
    };
    let fim = fim_f as usize;
    if fim < su.len() {
        return Ok(Valor::FALSE);
    }
    Ok(Valor::bool(casa_em(&tu, &su, fim - su.len())))
}

fn includes(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "includes")?;
    let v0 = m.arg(a, 0);
    let s = texto_de_busca(m, v0, "includes")?;
    let v1 = m.arg(a, 1);
    let p = m.para_inteiro(v1)?;
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    let ini = limite(p, tu.len() as f64) as usize;
    Ok(Valor::bool(procurar(&tu, &su, ini).is_some()))
}

fn index_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "indexOf")?;
    let v0 = m.arg(a, 0);
    let s = m.para_texto(v0)?;
    let v1 = m.arg(a, 1);
    let p = m.para_inteiro(v1)?;
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    let ini = limite(p, tu.len() as f64) as usize;
    let r = procurar(&tu, &su, ini).map_or(-1.0, |i| i as f64);
    Ok(m.numero(r))
}

fn last_index_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "lastIndexOf")?;
    let v0 = m.arg(a, 0);
    let s = m.para_texto(v0)?;
    // Posição NaN (ou ausente) procura a partir do fim.
    let v1 = m.arg(a, 1);
    let x = m.para_numero(v1)?;
    let p = if x.is_nan() { f64::INFINITY } else { x.trunc() };
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    let n = tu.len();
    if su.len() > n {
        return Ok(m.numero(-1.0));
    }
    let mut i = (limite(p, n as f64) as usize).min(n - su.len());
    let r = loop {
        if casa_em(&tu, &su, i) {
            break i as f64;
        }
        if i == 0 {
            break -1.0;
        }
        i -= 1;
    };
    Ok(m.numero(r))
}

fn locale_compare(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "localeCompare")?;
    let v0 = m.arg(a, 0);
    let s = m.para_texto(v0)?;
    // shortcut: comparação por unidades UTF-16, sem ordem de locale.
    let r = match m.comparar_textos(t, s) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    };
    Ok(Valor::int(r))
}

fn normalize(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "normalize")?;
    let f = m.arg(a, 0);
    if f != Valor::UNDEFINED {
        let forma = m.para_texto_rust(f)?;
        if !["NFC", "NFD", "NFKC", "NFKD"].contains(&forma.as_str()) {
            return m.erro_alcance("The normalization form should be one of NFC, NFD, NFKC, NFKD.");
        }
    }
    // shortcut: devolve o mesmo texto, sem normalização Unicode real.
    Ok(t)
}

// ---------------------------------------------------------------- tamanho e recorte

/// `padStart` (no_inicio) e `padEnd`.
fn preencher(m: &mut Motor, this: Valor, a: Args, nome: &str, no_inicio: bool) -> Res<Valor> {
    let t = texto_this(m, this, nome)?;
    let v0 = m.arg(a, 0);
    let max = m.para_comprimento(v0)?;
    let n = f64::from(m.texto_len(t));
    if max <= n {
        return Ok(t);
    }
    let fv = m.arg(a, 1);
    let enche = if fv == Valor::UNDEFINED { m.texto(" ") } else { m.para_texto(fv)? };
    if m.texto_len(enche) == 0 {
        return Ok(t);
    }
    if max > f64::from(TEXTO_MAX) {
        return m.erro_alcance("Invalid string length");
    }
    let falta = (max - n) as usize;
    let f = m.texto_u16_de(enche);
    let pad: Vec<u16> = f.iter().cycle().take(falta).copied().collect();
    let p = m.texto_u16(&pad);
    Ok(if no_inicio { m.concatenar(p, t) } else { m.concatenar(t, p) })
}

fn pad_start(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    preencher(m, this, a, "padStart", true)
}

fn pad_end(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    preencher(m, this, a, "padEnd", false)
}

fn repeat(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "repeat")?;
    let v = m.arg(a, 0);
    let k = m.para_inteiro(v)?;
    if k < 0.0 || k.is_infinite() {
        let s = crate::numeros::numero_para_texto(k);
        return m.erro_alcance(&format!("Invalid count value: {s}"));
    }
    let n = f64::from(m.texto_len(t));
    if k == 0.0 || n == 0.0 {
        return Ok(Valor::VAZIA);
    }
    if n * k > f64::from(TEXTO_MAX) {
        return m.erro_alcance("Invalid string length");
    }
    let u = m.texto_u16_de(t);
    let r = u.repeat(k as usize);
    Ok(m.texto_u16(&r))
}

fn slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "slice")?;
    let n = f64::from(m.texto_len(t));
    let v0 = m.arg(a, 0);
    let p0 = m.para_inteiro(v0)?;
    let ini = relativo(p0, n);
    let v1 = m.arg(a, 1);
    let fim = if v1 == Valor::UNDEFINED {
        n
    } else {
        let p1 = m.para_inteiro(v1)?;
        relativo(p1, n)
    };
    if ini >= fim {
        return Ok(Valor::VAZIA);
    }
    Ok(m.subtexto(t, ini as u32, fim as u32))
}

fn substring(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "substring")?;
    let n = f64::from(m.texto_len(t));
    let v0 = m.arg(a, 0);
    let p0 = m.para_inteiro(v0)?;
    let ini = limite(p0, n);
    let v1 = m.arg(a, 1);
    let fim = if v1 == Valor::UNDEFINED {
        n
    } else {
        let p1 = m.para_inteiro(v1)?;
        limite(p1, n)
    };
    let (lo, hi) = if ini <= fim { (ini, fim) } else { (fim, ini) };
    Ok(m.subtexto(t, lo as u32, hi as u32))
}

fn substr(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "substr")?;
    let n = f64::from(m.texto_len(t));
    let v0 = m.arg(a, 0);
    let p0 = m.para_inteiro(v0)?;
    let ini = relativo(p0, n);
    let v1 = m.arg(a, 1);
    let comp = if v1 == Valor::UNDEFINED {
        n
    } else {
        m.para_inteiro(v1)?
    };
    let comp = limite(comp, n - ini);
    if comp <= 0.0 {
        return Ok(Valor::VAZIA);
    }
    Ok(m.subtexto(t, ini as u32, (ini + comp) as u32))
}

// ---------------------------------------------------------------- caixa, aparar, texto

fn caixa(m: &mut Motor, this: Valor, alta: bool) -> Res<Valor> {
    let nome = if alta { "toUpperCase" } else { "toLowerCase" };
    let t = texto_this(m, this, nome)?;
    let u = m.texto_u16_de(t);
    // shortcut: surrogate solto vira U+FFFD na caixa (o JS o preserva).
    let s = String::from_utf16_lossy(&u);
    let r = if alta { s.to_uppercase() } else { s.to_lowercase() };
    Ok(m.texto(&r))
}

fn to_lower(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    caixa(m, this, false)
}

fn to_upper(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    caixa(m, this, true)
}

fn aparar(m: &mut Motor, this: Valor, nome: &str, inicio: bool, fim: bool) -> Res<Valor> {
    let t = texto_this(m, this, nome)?;
    let u = m.texto_u16_de(t);
    let mut ini = 0usize;
    let mut end = u.len();
    if inicio {
        while ini < end && e_espaco(u[ini]) {
            ini += 1;
        }
    }
    if fim {
        while end > ini && e_espaco(u[end - 1]) {
            end -= 1;
        }
    }
    Ok(m.subtexto(t, ini as u32, end as u32))
}

fn trim(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    aparar(m, this, "trim", true, true)
}

fn trim_start(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    aparar(m, this, "trimStart", true, false)
}

fn trim_end(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    aparar(m, this, "trimEnd", false, true)
}

fn to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    texto_do_valor(m, this, "toString")
}

fn value_of(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    texto_do_valor(m, this, "valueOf")
}

fn is_well_formed(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "isWellFormed")?;
    let u = m.texto_u16_de(t);
    Ok(Valor::bool(!corrigir_surrogates(&u).1))
}

fn to_well_formed(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "toWellFormed")?;
    let u = m.texto_u16_de(t);
    let (out, mudou) = corrigir_surrogates(&u);
    if !mudou {
        return Ok(t);
    }
    Ok(m.texto_u16(&out))
}

/// `[Symbol.iterator]`: um iterador de array com um item por ponto de código.
fn iterador(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = texto_this(m, this, "[Symbol.iterator]")?;
    let u = m.texto_u16_de(t);
    let mut pedacos: Vec<Valor> = Vec::new();
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        let j = if (0xD800..0xDC00).contains(&c) && i + 1 < u.len() && (0xDC00..0xE000).contains(&u[i + 1]) {
            i + 2
        } else {
            i + 1
        };
        let pedaco = m.texto_u16(&u[i..j]);
        pedacos.push(pedaco);
        i = j;
    }
    let arr = m.array_de(&pedacos);
    let f = m.intr.array_valores;
    m.chamar(f, arr, &[])
}

// ---------------------------------------------------------------- split, replace, regex

/// `split`: usa `Symbol.split` do separador; senão divide o texto.
fn split(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if this.e_nulo_ou_indefinido() {
        return m.erro_tipo("String.prototype.split called on null or undefined");
    }
    let sep = m.arg(a, 0);
    let lim = m.arg(a, 1);
    let f = metodo_do_simbolo(m, sep, simb::SPLIT)?;
    if !f.e_nulo_ou_indefinido() {
        return m.chamar(f, sep, &[this, lim]);
    }
    let t = texto_this(m, this, "split")?;
    let lim_n = if lim == Valor::UNDEFINED { u32::MAX } else { m.para_uint32(lim)? };
    let max = lim_n as usize;
    let sep_txt = m.para_texto(sep)?;
    if max == 0 {
        return Ok(m.array_de(&[]));
    }
    if sep == Valor::UNDEFINED {
        return Ok(m.array_de(&[t]));
    }
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(sep_txt);
    let mut partes: Vec<(usize, usize)> = Vec::new();
    if su.is_empty() {
        for i in 0..tu.len().min(max) {
            partes.push((i, i + 1));
        }
    } else {
        let mut inicio = 0usize;
        let mut i = 0usize;
        while let Some(pos) = procurar(&tu, &su, i) {
            if partes.len() >= max {
                break;
            }
            partes.push((inicio, pos));
            inicio = pos + su.len();
            i = inicio;
        }
        if partes.len() < max {
            partes.push((inicio, tu.len()));
        }
    }
    let mut vals: Vec<Valor> = Vec::with_capacity(partes.len());
    for &(x, y) in &partes {
        let v = m.subtexto(t, x as u32, y as u32);
        vals.push(v);
    }
    Ok(m.array_de(&vals))
}

/// `replace`: usa `Symbol.replace` do padrão; senão troca a primeira ocorrência do texto.
fn replace(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if this.e_nulo_ou_indefinido() {
        return m.erro_tipo("String.prototype.replace called on null or undefined");
    }
    let pat = m.arg(a, 0);
    let rep = m.arg(a, 1);
    let f = metodo_do_simbolo(m, pat, simb::REPLACE)?;
    if !f.e_nulo_ou_indefinido() {
        return m.chamar(f, pat, &[this, rep]);
    }
    let t = texto_this(m, this, "replace")?;
    let s = m.para_texto(pat)?;
    let fn_sub = m.e_funcao(rep);
    let r_txt = if fn_sub { Valor::VAZIA } else { m.para_texto(rep)? };
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    let pos = match procurar(&tu, &su, 0) {
        Some(p) => p,
        None => return Ok(t),
    };
    let sub = if fn_sub {
        let pv = m.numero(pos as f64);
        let r = m.chamar(rep, Valor::UNDEFINED, &[s, pv, t])?;
        m.para_texto(r)?
    } else {
        let modelo = m.texto_u16_de(r_txt);
        let novo = substituicao(&modelo, &tu, pos, &su);
        m.texto_u16(&novo)
    };
    let antes = m.subtexto(t, 0, pos as u32);
    let depois = m.subtexto(t, (pos + su.len()) as u32, tu.len() as u32);
    let x = m.concatenar(antes, sub);
    Ok(m.concatenar(x, depois))
}

/// `replaceAll`: troca todas as ocorrências (regex precisa ser global).
fn replace_all(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if this.e_nulo_ou_indefinido() {
        return m.erro_tipo("String.prototype.replaceAll called on null or undefined");
    }
    let pat = m.arg(a, 0);
    let rep = m.arg(a, 1);
    if e_regex(m, pat) {
        let fl = m.obter(pat, ChaveProp::Nome(fixos::FLAGS))?;
        let fl = m.para_texto(fl)?;
        if !m.texto_u16_de(fl).contains(&u16::from(b'g')) {
            return m.erro_tipo("replaceAll must be called with a global RegExp");
        }
    }
    let f = metodo_do_simbolo(m, pat, simb::REPLACE)?;
    if !f.e_nulo_ou_indefinido() {
        return m.chamar(f, pat, &[this, rep]);
    }
    let t = texto_this(m, this, "replaceAll")?;
    let s = m.para_texto(pat)?;
    let fn_sub = m.e_funcao(rep);
    let r_txt = if fn_sub { Valor::VAZIA } else { m.para_texto(rep)? };
    let tu = m.texto_u16_de(t);
    let su = m.texto_u16_de(s);
    // Ocorrências sem sobreposição; padrão vazio casa em cada posição.
    let adv = su.len().max(1);
    let mut posicoes: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while let Some(pos) = procurar(&tu, &su, i) {
        posicoes.push(pos);
        i = pos + adv;
        if i > tu.len() {
            break;
        }
    }
    if posicoes.is_empty() {
        return Ok(t);
    }
    let modelo = if fn_sub { Vec::new() } else { m.texto_u16_de(r_txt) };
    let mut partes: Vec<Valor> = Vec::new();
    let mut fim_ant = 0usize;
    for &pos in &posicoes {
        let antes = m.subtexto(t, fim_ant as u32, pos as u32);
        partes.push(antes);
        let sub = if fn_sub {
            let pv = m.numero(pos as f64);
            let r = m.chamar(rep, Valor::UNDEFINED, &[s, pv, t])?;
            m.para_texto(r)?
        } else {
            let novo = substituicao(&modelo, &tu, pos, &su);
            m.texto_u16(&novo)
        };
        partes.push(sub);
        fim_ant = pos + su.len();
    }
    partes.push(m.subtexto(t, fim_ant as u32, tu.len() as u32));
    let mut acc = Valor::VAZIA;
    for p in partes {
        acc = m.concatenar(acc, p);
    }
    Ok(acc)
}

/// `match`, `matchAll` e `search`: usam o método do símbolo do argumento; sem ele, criam um
/// RegExp com o argumento (com "g" em `matchAll`) e chamam o método dele.
fn delega_regex(m: &mut Motor, this: Valor, a: Args, s: Chave, nome: &str, global: bool) -> Res<Valor> {
    if this.e_nulo_ou_indefinido() {
        return m.erro_tipo(&format!("String.prototype.{nome} called on null or undefined"));
    }
    let rx = m.arg(a, 0);
    let f = metodo_do_simbolo(m, rx, s)?;
    if !f.e_nulo_ou_indefinido() {
        return m.chamar(f, rx, &[this]);
    }
    if global && e_regex(m, rx) {
        let fl = m.obter(rx, ChaveProp::Nome(fixos::FLAGS))?;
        let fl = m.para_texto(fl)?;
        if !m.texto_u16_de(fl).contains(&u16::from(b'g')) {
            return m.erro_tipo(&format!("String.prototype.{nome} called with a non-global RegExp argument"));
        }
    }
    let t = texto_this(m, this, nome)?;
    let g = m.intr.global;
    let ck = m.chave("RegExp");
    let ctor = m.obter(g, ChaveProp::Nome(ck))?;
    let args = if global {
        let gv = m.texto("g");
        vec![rx, gv]
    } else {
        vec![rx]
    };
    let obj = m.construir(ctor, &args, ctor)?;
    let ms = metodo_do_simbolo(m, obj, s)?;
    m.chamar(ms, obj, &[t])
}

fn regex_match(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    delega_regex(m, this, a, simb::MATCH, "match", false)
}

fn regex_match_all(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    delega_regex(m, this, a, simb::MATCH_ALL, "matchAll", true)
}

fn regex_search(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    delega_regex(m, this, a, simb::SEARCH, "search", false)
}
