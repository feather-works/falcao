//! `Math` (§21.3): funções, constantes e `Symbol.toStringTag`.

use crate::formas::attr;
use crate::funcoes::{Args, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::operadores;
use crate::valor::Valor;
use std::cell::Cell;
use std::f64::consts::{E, FRAC_1_SQRT_2, LN_10, LN_2, LOG10_E, LOG2_E, PI, SQRT_2};

thread_local! {
    /// Estado do gerador `xorshift64*` de `Math.random`.
    static ESTADO: Cell<u64> = Cell::new(0);
}

/// Funções de um argumento que só chamam uma função de `f64`.
macro_rules! unarias {
    ($($nome:ident => $f:expr),* $(,)?) => {
        $(
            fn $nome(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
                let v = m.arg(a, 0);
                let x = m.para_numero(v)?;
                let f: fn(f64) -> f64 = $f;
                Ok(m.numero(f(x)))
            }
        )*
    };
}

unarias! {
    abs => f64::abs,
    acos => f64::acos,
    acosh => f64::acosh,
    asin => f64::asin,
    asinh => f64::asinh,
    atan => f64::atan,
    atanh => f64::atanh,
    cbrt => f64::cbrt,
    ceil => f64::ceil,
    cos => f64::cos,
    cosh => f64::cosh,
    exp => f64::exp,
    expm1 => f64::exp_m1,
    floor => f64::floor,
    log => f64::ln,
    log1p => f64::ln_1p,
    log10 => f64::log10,
    log2 => f64::log2,
    sin => f64::sin,
    sinh => f64::sinh,
    sqrt => f64::sqrt,
    tan => f64::tan,
    tanh => f64::tanh,
    trunc => f64::trunc,
}

pub(super) fn instalar(m: &mut Motor) {
    let math = m.objeto_vazio();
    m.metodo(math, "abs", 1, abs);
    m.metodo(math, "acos", 1, acos);
    m.metodo(math, "acosh", 1, acosh);
    m.metodo(math, "asin", 1, asin);
    m.metodo(math, "asinh", 1, asinh);
    m.metodo(math, "atan", 1, atan);
    m.metodo(math, "atanh", 1, atanh);
    m.metodo(math, "atan2", 2, atan2);
    m.metodo(math, "cbrt", 1, cbrt);
    m.metodo(math, "ceil", 1, ceil);
    m.metodo(math, "clz32", 1, clz32);
    m.metodo(math, "cos", 1, cos);
    m.metodo(math, "cosh", 1, cosh);
    m.metodo(math, "exp", 1, exp);
    m.metodo(math, "expm1", 1, expm1);
    m.metodo(math, "floor", 1, floor);
    m.metodo(math, "fround", 1, fround);
    m.metodo(math, "hypot", 2, hypot);
    m.metodo(math, "imul", 2, imul);
    m.metodo(math, "log", 1, log);
    m.metodo(math, "log1p", 1, log1p);
    m.metodo(math, "log10", 1, log10);
    m.metodo(math, "log2", 1, log2);
    m.metodo(math, "max", 2, max);
    m.metodo(math, "min", 2, min);
    m.metodo(math, "pow", 2, pow);
    m.metodo(math, "random", 0, random);
    m.metodo(math, "round", 1, round);
    m.metodo(math, "sign", 1, sign);
    m.metodo(math, "sin", 1, sin);
    m.metodo(math, "sinh", 1, sinh);
    m.metodo(math, "sqrt", 1, sqrt);
    m.metodo(math, "tan", 1, tan);
    m.metodo(math, "tanh", 1, tanh);
    m.metodo(math, "trunc", 1, trunc);

    let consts: [(&str, f64); 8] = [
        ("E", E),
        ("LN10", LN_10),
        ("LN2", LN_2),
        ("LOG10E", LOG10_E),
        ("LOG2E", LOG2_E),
        ("PI", PI),
        ("SQRT1_2", FRAC_1_SQRT_2),
        ("SQRT2", SQRT_2),
    ];
    for &(nome, x) in consts.iter() {
        let v = m.numero(x);
        let c = m.chave(nome);
        m.definir_propria(math, c, v, 0);
    }
    let tag = m.texto("Math");
    m.definir_propria(math, simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
    let g = m.intr.global;
    let c = m.chave("Math");
    m.definir_propria(g, c, math, attr::METODO);
}

// ---------------------------------------------------------------- funções com nome próprio

fn atan2(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (va, vb) = (m.arg(a, 0), m.arg(a, 1));
    let y = m.para_numero(va)?;
    let x = m.para_numero(vb)?;
    Ok(m.numero(y.atan2(x)))
}

fn pow(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (va, vb) = (m.arg(a, 0), m.arg(a, 1));
    let base = m.para_numero(va)?;
    let expoente = m.para_numero(vb)?;
    Ok(m.numero(operadores::potencia(base, expoente)))
}

fn imul(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (va, vb) = (m.arg(a, 0), m.arg(a, 1));
    let x = m.para_int32(va)?;
    let y = m.para_int32(vb)?;
    Ok(m.numero(f64::from(x.wrapping_mul(y))))
}

fn clz32(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = m.para_uint32(v)?;
    Ok(Valor::int(x.leading_zeros() as i32))
}

fn fround(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = m.para_numero(v)?;
    Ok(m.numero(f64::from(x as f32)))
}

fn sign(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = m.para_numero(v)?;
    let r = if x.is_nan() || x == 0.0 {
        x
    } else if x > 0.0 {
        1.0
    } else {
        -1.0
    };
    Ok(m.numero(r))
}

/// `Math.round` do JS: x.5 vai para cima (Math.round(-2.5) = -2), e -0 é preservado.
fn arredondar(x: f64) -> f64 {
    if !x.is_finite() || x.fract() == 0.0 {
        return x;
    }
    if x < 0.0 && x >= -0.5 {
        return -0.0;
    }
    let f = x.floor();
    if x - f >= 0.5 {
        f + 1.0
    } else {
        f
    }
}

fn round(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let x = m.para_numero(v)?;
    Ok(m.numero(arredondar(x)))
}

/// `max` (`maximo`) e `min`: todos os argumentos são convertidos; NaN domina; -0 < +0.
fn extremo(m: &mut Motor, a: Args, maximo: bool) -> Res<Valor> {
    let mut r = if maximo { f64::NEG_INFINITY } else { f64::INFINITY };
    let mut nan = false;
    for i in 0..a.n {
        let v = m.arg(a, i);
        let x = m.para_numero(v)?;
        if x.is_nan() {
            nan = true;
        } else if maximo {
            if x > r || (x == 0.0 && r == 0.0 && x.is_sign_positive()) {
                r = x;
            }
        } else if x < r || (x == 0.0 && r == 0.0 && x.is_sign_negative()) {
            r = x;
        }
    }
    Ok(m.numero(if nan { f64::NAN } else { r }))
}

fn max(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    extremo(m, a, true)
}

fn min(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    extremo(m, a, false)
}

fn hypot(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let mut xs: Vec<f64> = Vec::with_capacity(a.n as usize);
    for i in 0..a.n {
        let v = m.arg(a, i);
        xs.push(m.para_numero(v)?);
    }
    if xs.iter().any(|x| x.is_infinite()) {
        return Ok(m.numero(f64::INFINITY));
    }
    if xs.iter().any(|x| x.is_nan()) {
        return Ok(m.numero(f64::NAN));
    }
    let maior = xs.iter().fold(0.0f64, |acc, x| acc.max(x.abs()));
    if maior == 0.0 {
        return Ok(m.numero(0.0));
    }
    // Escala pelo maior valor para não estourar na soma dos quadrados.
    let soma: f64 = xs.iter().map(|x| (x / maior) * (x / maior)).sum();
    Ok(m.numero(maior * soma.sqrt()))
}

/// Semente tirada do relógio (nunca zero).
fn semente() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    (t ^ 0x9E37_79B9_7F4A_7C15) | 1
}

fn random(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let bits = ESTADO.with(|e| {
        let mut s = e.get();
        if s == 0 {
            s = semente();
        }
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        e.set(s);
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    });
    // 53 bits superiores dão um f64 em [0, 1).
    Ok(m.numero((bits >> 11) as f64 / (1u64 << 53) as f64))
}
