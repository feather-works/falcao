//! `Number` e `Boolean` (§21.1 e §20.3), e as funções globais de número e de URI
//! (`parseInt`, `parseFloat`, `isNaN`, `isFinite`, `encodeURIComponent`, `escape`...).

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::intrinsecos::TipoErro;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::numeros;
use crate::textos::ChaveProp;
use crate::valor::Valor;

/// Marcas que `encodeURIComponent` deixa sem escape (além dos alfanuméricos).
const MARCAS: &str = "-_.!~*'()";
/// Reservados de URI: `encodeURI` os deixa, e `decodeURI` não os decodifica.
const RESERVADOS: &str = ";/?:@&=+$,#";
/// Maior inteiro seguro (2^53 - 1).
const MAX_SEGURO: f64 = 9_007_199_254_740_991.0;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;

    // Number
    let np = m.intr.numero_proto;
    let nctor = m.nativa("Number", 1, num_construtor, true);
    m.definir_propria(nctor, fixos::PROTOTYPE, np, 0);
    m.definir_propria(np, fixos::CONSTRUCTOR, nctor, attr::METODO);
    let consts: [(&str, f64); 8] = [
        ("MAX_SAFE_INTEGER", MAX_SEGURO),
        ("MIN_SAFE_INTEGER", -MAX_SEGURO),
        ("EPSILON", f64::EPSILON),
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", f64::from_bits(1)),
        ("NaN", f64::NAN),
        ("POSITIVE_INFINITY", f64::INFINITY),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
    ];
    for &(nome, x) in consts.iter() {
        let v = m.numero(x);
        expor(m, nctor, nome, v, 0);
    }
    m.metodo(nctor, "isFinite", 1, num_is_finite);
    m.metodo(nctor, "isInteger", 1, num_is_integer);
    m.metodo(nctor, "isNaN", 1, num_is_nan);
    m.metodo(nctor, "isSafeInteger", 1, num_is_safe_integer);
    // parseFloat e parseInt são o mesmo objeto no global e em Number.
    let pf = m.nativa("parseFloat", 1, parse_float_js, false);
    let pi = m.nativa("parseInt", 2, parse_int_js, false);
    expor(m, nctor, "parseFloat", pf, attr::METODO);
    expor(m, nctor, "parseInt", pi, attr::METODO);
    expor(m, g, "parseFloat", pf, attr::METODO);
    expor(m, g, "parseInt", pi, attr::METODO);
    m.metodo(np, "toString", 1, num_to_string);
    m.metodo(np, "toFixed", 1, num_to_fixed);
    m.metodo(np, "toExponential", 1, num_to_exponential);
    m.metodo(np, "toPrecision", 1, num_to_precision);
    m.metodo(np, "toLocaleString", 0, num_to_locale);
    m.metodo(np, "valueOf", 0, num_value_of);
    expor(m, g, "Number", nctor, attr::METODO);

    // Boolean
    let bp = m.intr.booleano_proto;
    let bctor = m.nativa("Boolean", 1, bool_construtor, true);
    m.definir_propria(bctor, fixos::PROTOTYPE, bp, 0);
    m.definir_propria(bp, fixos::CONSTRUCTOR, bctor, attr::METODO);
    m.metodo(bp, "toString", 0, bool_to_string);
    m.metodo(bp, "valueOf", 0, bool_value_of);
    expor(m, g, "Boolean", bctor, attr::METODO);

    // Funções globais.
    m.metodo(g, "isNaN", 1, isnan_global);
    m.metodo(g, "isFinite", 1, isfinite_global);
    m.metodo(g, "encodeURIComponent", 1, encode_uri_component);
    m.metodo(g, "encodeURI", 1, encode_uri);
    m.metodo(g, "decodeURIComponent", 1, decode_uri_component);
    m.metodo(g, "decodeURI", 1, decode_uri);
    m.metodo(g, "escape", 1, escape);
    m.metodo(g, "unescape", 1, unescape);
}

// ---------------------------------------------------------------- auxiliares

/// Define `obj[nome] = v` com os atributos dados.
fn expor(m: &mut Motor, obj: Valor, nome: &str, v: Valor, attrs: u8) {
    let c = m.chave(nome);
    m.definir_propria(obj, c, v, attrs);
}

/// Valor interno de um objeto caixa (`None` se `v` não é caixa).
fn interno_de_caixa(m: &Motor, v: Valor) -> Option<Valor> {
    if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Caixa {
        Some(Valor(m.interno(v, 0)))
    } else {
        None
    }
}

/// Objeto caixa novo, com o protótipo de `nt` (ou `padrao`) e o primitivo `v`.
fn nova_caixa(m: &mut Motor, nt: Valor, v: Valor, padrao: Valor) -> Res<Valor> {
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = if m.e_objeto(p) { p } else { padrao };
    let f = m.forma_para(Classe::Caixa, p, 0);
    let o = m.novo_objeto(f);
    m.por_interno_valor(o, 0, v);
    Ok(o)
}

/// `thisNumberValue`: número ou caixa de número.
fn numero_this(m: &mut Motor, this: Valor, nome: &str) -> Res<f64> {
    if m.e_numero(this) {
        return Ok(m.f64_de(this));
    }
    if let Some(p) = interno_de_caixa(m, this) {
        if m.e_numero(p) {
            return Ok(m.f64_de(p));
        }
    }
    m.erro_tipo(&format!("Number.prototype.{nome} requires that 'this' be a Number"))
}

/// `thisBooleanValue`: booleano ou caixa de booleano.
fn bool_this(m: &mut Motor, this: Valor, nome: &str) -> Res<bool> {
    let v = if this == Valor::TRUE || this == Valor::FALSE {
        this
    } else {
        interno_de_caixa(m, this).unwrap_or(Valor::UNDEFINED)
    };
    if v == Valor::TRUE {
        Ok(true)
    } else if v == Valor::FALSE {
        Ok(false)
    } else {
        m.erro_tipo(&format!("Boolean.prototype.{nome} requires that 'this' be a Boolean"))
    }
}

fn inteiro_finito(x: f64) -> bool {
    x.is_finite() && x.trunc() == x
}

/// `malformado`: `URIError: URI malformed`.
fn malformado<T>(m: &mut Motor) -> Res<T> {
    Err(m.erro(TipoErro::Uri, "URI malformed"))
}

fn push_pct(out: &mut String, b: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[usize::from(b >> 4)]));
    out.push(char::from(HEX[usize::from(b & 15)]));
}

/// Valor de uma sequência de dígitos hexadecimais (`None` se algum caractere não é hex).
fn valor_hex(s: &[u16]) -> Option<u32> {
    s.iter().try_fold(0u32, |acc, &c| {
        let d = match c {
            0x30..=0x39 => c - 0x30,
            0x41..=0x46 => c - 0x41 + 10,
            0x61..=0x66 => c - 0x61 + 10,
            _ => return None,
        };
        Some(acc * 16 + u32::from(d))
    })
}

/// Byte do escape `%XX` que começa em `i`.
fn pct(u: &[u16], i: usize) -> Option<u8> {
    if u.get(i) != Some(&u16::from(b'%')) || i + 2 >= u.len() {
        return None;
    }
    valor_hex(&u[i + 1..i + 3]).map(|v| v as u8)
}

// ---------------------------------------------------------------- Number

fn num_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let v = if a.n == 0 {
        Valor::int(0)
    } else {
        let x = m.arg(a, 0);
        let p = m.para_numerico(x)?;
        if m.e_bigint(p) {
            let x = m.ler_bigint(p).para_f64();
            m.numero(x)
        } else {
            p
        }
    };
    if nt == Valor::UNDEFINED {
        return Ok(v);
    }
    let padrao = m.intr.numero_proto;
    nova_caixa(m, nt, v, padrao)
}

fn num_is_finite(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    Ok(Valor::bool(m.e_numero(v) && m.f64_de(v).is_finite()))
}

fn num_is_integer(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = if m.e_numero(v) { m.f64_de(v) } else { f64::NAN };
    Ok(Valor::bool(inteiro_finito(x)))
}

fn num_is_nan(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    Ok(Valor::bool(m.e_numero(v) && m.f64_de(v).is_nan()))
}

fn num_is_safe_integer(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = if m.e_numero(v) { m.f64_de(v) } else { f64::NAN };
    Ok(Valor::bool(inteiro_finito(x) && x.abs() <= MAX_SEGURO))
}

fn isnan_global(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = m.para_numero(v)?;
    Ok(Valor::bool(x.is_nan()))
}

fn isfinite_global(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = m.para_numero(v)?;
    Ok(Valor::bool(x.is_finite()))
}

fn parse_float_js(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let s = m.para_texto(v)?;
    let u = m.texto_u16_de(s);
    let x = numeros::parse_float(&u);
    Ok(m.numero(x))
}

fn parse_int_js(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let s = m.para_texto(v)?;
    let r = m.arg(a, 1);
    let raiz = m.para_int32(r)?;
    let u = m.texto_u16_de(s);
    let x = numeros::parse_int(&u, raiz);
    Ok(m.numero(x))
}

fn num_to_string(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let x = numero_this(m, this, "toString")?;
    let rv = m.arg(a, 0);
    let raiz = if rv == Valor::UNDEFINED { 10.0 } else { m.para_inteiro(rv)? };
    if !(2.0..=36.0).contains(&raiz) {
        return m.erro_alcance("toString() radix must be between 2 and 36");
    }
    Ok(m.numero_texto_raiz(x, raiz as u32))
}

fn num_to_fixed(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let x = numero_this(m, this, "toFixed")?;
    let dv = m.arg(a, 0);
    let d = m.para_inteiro(dv)?;
    if !(0.0..=100.0).contains(&d) {
        return m.erro_alcance("toFixed() digits argument must be between 0 and 100");
    }
    if !x.is_finite() || x.abs() >= 1e21 {
        return Ok(m.numero_para_texto(x));
    }
    // -0 vira 0, como no JS.
    let x = if x == 0.0 { 0.0 } else { x };
    let s = numeros::para_fixo(x, d as u32);
    Ok(m.texto(&s))
}

fn num_to_exponential(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let x = numero_this(m, this, "toExponential")?;
    let fv = m.arg(a, 0);
    let f = m.para_inteiro(fv)?;
    if !x.is_finite() {
        return Ok(m.numero_para_texto(x));
    }
    let casas = if fv == Valor::UNDEFINED {
        None
    } else {
        if !(0.0..=100.0).contains(&f) {
            return m.erro_alcance("toExponential() argument must be between 0 and 100");
        }
        Some(f as u32)
    };
    let s = numeros::para_exponencial(x, casas);
    Ok(m.texto(&s))
}

fn num_to_precision(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let x = numero_this(m, this, "toPrecision")?;
    let pv = m.arg(a, 0);
    if pv == Valor::UNDEFINED {
        return Ok(m.numero_para_texto(x));
    }
    let p = m.para_inteiro(pv)?;
    if !x.is_finite() {
        return Ok(m.numero_para_texto(x));
    }
    if !(1.0..=100.0).contains(&p) {
        return m.erro_alcance("toPrecision() argument must be between 1 and 100");
    }
    let s = numeros::para_precisao(x, p as u32);
    Ok(m.texto(&s))
}

/// Formato pt-BR simples: "." separa milhar, "," separa decimal, até 3 casas.
fn num_to_locale(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let x = numero_this(m, this, "toLocaleString")?;
    if x.is_nan() {
        return Ok(m.texto("NaN"));
    }
    if x.is_infinite() {
        return Ok(m.texto(if x > 0.0 { "∞" } else { "-∞" }));
    }
    // shortcut: acima de 1e21 não agrupa (sai na notação do toString).
    if x.abs() >= 1e21 {
        return Ok(m.numero_para_texto(x));
    }
    let fixo = numeros::para_fixo(x.abs(), 3);
    let (inteira, frac) = match fixo.split_once('.') {
        Some((i, f)) => (i, f.trim_end_matches('0')),
        None => (fixo.as_str(), ""),
    };
    let mut r = String::new();
    // Negativo com parte zerada sai como "-0", como no ICU.
    if x < 0.0 {
        r.push('-');
    }
    let n = inteira.len();
    for (k, c) in inteira.chars().enumerate() {
        if k > 0 && (n - k) % 3 == 0 {
            r.push('.');
        }
        r.push(c);
    }
    if !frac.is_empty() {
        r.push(',');
        r.push_str(frac);
    }
    Ok(m.texto(&r))
}

fn num_value_of(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let x = numero_this(m, this, "valueOf")?;
    Ok(m.numero(x))
}

// ---------------------------------------------------------------- Boolean

fn bool_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let x = m.arg(a, 0);
    let b = Valor::bool(m.verdadeiro(x));
    if nt == Valor::UNDEFINED {
        return Ok(b);
    }
    let padrao = m.intr.booleano_proto;
    nova_caixa(m, nt, b, padrao)
}

fn bool_to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = bool_this(m, this, "toString")?;
    Ok(m.texto(if b { "true" } else { "false" }))
}

fn bool_value_of(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = bool_this(m, this, "valueOf")?;
    Ok(Valor::bool(b))
}

// ---------------------------------------------------------------- URI e escape

fn encode_uri_component(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    codificar(m, v, "")
}

fn encode_uri(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    codificar(m, v, RESERVADOS)
}

fn decode_uri_component(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    decodificar(m, v, "")
}

fn decode_uri(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    decodificar(m, v, RESERVADOS)
}

/// Codifica em UTF-8 com `%XX`. `extra` são caracteres ASCII que ficam sem escape.
fn codificar(m: &mut Motor, v: Valor, extra: &str) -> Res<Valor> {
    let s = m.para_texto(v)?;
    let u = m.texto_u16_de(s);
    let mut out = String::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        let c = u[i];
        if c < 128 {
            let ch = c as u8 as char;
            if ch.is_ascii_alphanumeric() || MARCAS.contains(ch) || extra.contains(ch) {
                out.push(ch);
            } else {
                push_pct(&mut out, c as u8);
            }
            i += 1;
            continue;
        }
        let cp = if (0xDC00..0xE000).contains(&c) {
            return malformado(m);
        } else if (0xD800..0xDC00).contains(&c) {
            match u.get(i + 1) {
                Some(&d) if (0xDC00..0xE000).contains(&d) => {
                    i += 1;
                    0x10000 + ((u32::from(c) - 0xD800) << 10) + (u32::from(d) - 0xDC00)
                }
                _ => return malformado(m),
            }
        } else {
            u32::from(c)
        };
        i += 1;
        if let Some(ch) = char::from_u32(cp) {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                push_pct(&mut out, b);
            }
        }
    }
    Ok(m.texto(&out))
}

/// Decodifica `%XX` (UTF-8). Os reservados em `reservados` continuam como estavam escritos.
fn decodificar(m: &mut Motor, v: Valor, reservados: &str) -> Res<Valor> {
    let s = m.para_texto(v)?;
    let u = m.texto_u16_de(s);
    let mut out: Vec<u16> = Vec::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        if u[i] != u16::from(b'%') {
            out.push(u[i]);
            i += 1;
            continue;
        }
        let Some(b) = pct(&u, i) else {
            return malformado(m);
        };
        if b < 0x80 {
            if reservados.as_bytes().contains(&b) {
                out.extend_from_slice(&u[i..i + 3]);
            } else {
                out.push(u16::from(b));
            }
            i += 3;
            continue;
        }
        let n: usize = if (b & 0xE0) == 0xC0 {
            2
        } else if (b & 0xF0) == 0xE0 {
            3
        } else if (b & 0xF8) == 0xF0 {
            4
        } else {
            return malformado(m);
        };
        let mut bytes = vec![b];
        let mut j = i + 3;
        for _ in 1..n {
            match pct(&u, j) {
                Some(c) if (c & 0xC0) == 0x80 => bytes.push(c),
                _ => return malformado(m),
            }
            j += 3;
        }
        match std::str::from_utf8(&bytes) {
            Ok(t) => out.extend(t.encode_utf16()),
            Err(_) => return malformado(m),
        }
        i = j;
    }
    Ok(m.texto_u16(&out))
}

/// `escape` (Anexo B): alfanuméricos e `@*_+-./` ficam; o resto vira `%XX` ou `%uXXXX`.
fn escape(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let s = m.para_texto(v)?;
    let u = m.texto_u16_de(s);
    let mut out = String::with_capacity(u.len());
    for &c in &u {
        if c < 128 && ((c as u8).is_ascii_alphanumeric() || b"@*_+-./".contains(&(c as u8))) {
            out.push(c as u8 as char);
        } else if c < 256 {
            push_pct(&mut out, c as u8);
        } else {
            out.push_str(&format!("%u{c:04X}"));
        }
    }
    Ok(m.texto(&out))
}

/// `unescape` (Anexo B): desfaz `%uXXXX` e `%XX`; o resto fica como está.
fn unescape(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let s = m.para_texto(v)?;
    let u = m.texto_u16_de(s);
    let mut out: Vec<u16> = Vec::with_capacity(u.len());
    let mut i = 0;
    while i < u.len() {
        if u[i] == u16::from(b'%') {
            if i + 5 < u.len() && u[i + 1] == u16::from(b'u') {
                if let Some(cu) = valor_hex(&u[i + 2..i + 6]) {
                    out.push(cu as u16);
                    i += 6;
                    continue;
                }
            }
            if let Some(b) = pct(&u, i) {
                out.push(u16::from(b));
                i += 3;
                continue;
            }
        }
        out.push(u[i]);
        i += 1;
    }
    Ok(m.texto_u16(&out))
}
