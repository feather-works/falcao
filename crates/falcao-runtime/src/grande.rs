//! Aritmética de inteiros grandes do BigInt (§6.1.6.2), escrita por nós: magnitude em
//! palavras de 32 bits (a menos significativa primeiro) e sinal à parte, como o objeto
//! `formas::Tipo::BigInt` guarda na gaiola. Multiplicação escolar e divisão longa (o quociente
//! de cada palavra é estimado pelas duas palavras do topo e corrigido).
//!
//! Tudo aqui é Rust puro, sem motor: o `Motor` só lê e grava `Grande` na gaiola.

use std::cmp::Ordering;

/// Maior BigInt aceito, em bits (passar disso dá RangeError, como nos outros motores).
pub const MAX_BITS: u64 = 1 << 30;

/// Inteiro de precisão arbitrária. Sempre normalizado: sem palavra zero no topo, e o zero
/// nunca é negativo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Grande {
    pub neg: bool,
    pub mag: Vec<u32>,
}

// ---------------------------------------------------------------- magnitudes

fn aparar(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn somar_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (a, b) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut r = Vec::with_capacity(a.len() + 1);
    let mut vai = 0u64;
    for i in 0..a.len() {
        let s = u64::from(a[i]) + u64::from(*b.get(i).unwrap_or(&0)) + vai;
        r.push(s as u32);
        vai = s >> 32;
    }
    if vai != 0 {
        r.push(vai as u32);
    }
    r
}

/// `a - b`, com `a >= b`.
fn subtrair_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut r = Vec::with_capacity(a.len());
    let mut emprestimo = 0i64;
    for i in 0..a.len() {
        let t = i64::from(a[i]) - i64::from(*b.get(i).unwrap_or(&0)) - emprestimo;
        r.push(t as u32);
        emprestimo = i64::from(t < 0);
    }
    aparar(&mut r);
    r
}

fn multiplicar_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut r = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut vai = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = u64::from(x) * u64::from(y) + u64::from(r[i + j]) + vai;
            r[i + j] = t as u32;
            vai = t >> 32;
        }
        r[i + b.len()] = vai as u32;
    }
    aparar(&mut r);
    r
}

/// Divide por uma palavra: (quociente, resto).
fn dividir_palavra(a: &[u32], d: u32) -> (Vec<u32>, u32) {
    let mut q = vec![0u32; a.len()];
    let mut resto = 0u64;
    for i in (0..a.len()).rev() {
        let atual = (resto << 32) | u64::from(a[i]);
        q[i] = (atual / u64::from(d)) as u32;
        resto = atual % u64::from(d);
    }
    aparar(&mut q);
    (q, resto as u32)
}

fn desl_esq_mag(a: &[u32], k: u64) -> Vec<u32> {
    if a.is_empty() {
        return Vec::new();
    }
    let (palavras, bits) = ((k / 32) as usize, (k % 32) as u32);
    let mut r = vec![0u32; palavras];
    if bits == 0 {
        r.extend_from_slice(a);
    } else {
        let mut vai = 0u32;
        for &x in a {
            r.push((x << bits) | vai);
            vai = x >> (32 - bits);
        }
        r.push(vai);
    }
    aparar(&mut r);
    r
}

/// `a >> k` e se algum bit 1 caiu fora.
fn desl_dir_mag(a: &[u32], k: u64) -> (Vec<u32>, bool) {
    let palavras = (k / 32) as usize;
    if palavras >= a.len() {
        return (Vec::new(), !a.is_empty());
    }
    let bits = (k % 32) as u32;
    let mut perdeu = a[..palavras].iter().any(|&x| x != 0);
    let resto = &a[palavras..];
    let mut r = Vec::with_capacity(resto.len());
    if bits == 0 {
        r.extend_from_slice(resto);
    } else {
        perdeu |= resto[0] & ((1 << bits) - 1) != 0;
        for i in 0..resto.len() {
            let alto = resto.get(i + 1).map_or(0, |&x| x << (32 - bits));
            r.push((resto[i] >> bits) | alto);
        }
    }
    aparar(&mut r);
    (r, perdeu)
}

/// Divisão longa: (quociente, resto) das magnitudes; `b` não é zero.
fn dividir_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if cmp_mag(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let (q, r) = dividir_palavra(a, b[0]);
        let mut r = vec![r];
        aparar(&mut r);
        return (q, r);
    }
    // Normaliza: o topo do divisor fica com o bit alto ligado, para a estimativa errar por pouco.
    let s = b[b.len() - 1].leading_zeros();
    let v = desl_esq_mag(b, u64::from(s));
    let mut u = desl_esq_mag(a, u64::from(s));
    u.resize(a.len() + 1, 0);
    let n = v.len();
    let m = a.len() - n;
    let base = 1u64 << 32;
    let (v1, v2) = (u64::from(v[n - 1]), u64::from(v[n - 2]));
    let mut q = vec![0u32; m + 1];
    for j in (0..=m).rev() {
        let topo = (u64::from(u[j + n]) << 32) | u64::from(u[j + n - 1]);
        let mut qq = topo / v1;
        let mut rr = topo % v1;
        // Estimativa pelas duas palavras do topo: no máximo 2 a mais que o certo.
        while qq >= base || qq * v2 > ((rr << 32) | u64::from(u[j + n - 2])) {
            qq -= 1;
            rr += v1;
            if rr >= base {
                break;
            }
        }
        // u[j..j+n+1] -= qq * v
        let mut emprestimo = 0i64;
        let mut vai = 0u64;
        for i in 0..n {
            let p = qq * u64::from(v[i]) + vai;
            vai = p >> 32;
            let t = i64::from(u[i + j]) - emprestimo - (p & 0xFFFF_FFFF) as i64;
            u[i + j] = t as u32;
            emprestimo = i64::from(t < 0);
        }
        let t = i64::from(u[j + n]) - emprestimo - vai as i64;
        u[j + n] = t as u32;
        if t < 0 {
            // Passou de um: devolve o divisor.
            qq -= 1;
            let mut vai = 0u64;
            for i in 0..n {
                let s = u64::from(u[i + j]) + u64::from(v[i]) + vai;
                u[i + j] = s as u32;
                vai = s >> 32;
            }
            u[j + n] = u[j + n].wrapping_add(vai as u32);
        }
        q[j] = qq as u32;
    }
    aparar(&mut q);
    u.truncate(n);
    let (r, _) = desl_dir_mag(&u, u64::from(s));
    (q, r)
}

// ---------------------------------------------------------------- Grande

impl Grande {
    pub fn novo(neg: bool, mut mag: Vec<u32>) -> Self {
        aparar(&mut mag);
        let neg = neg && !mag.is_empty();
        Grande { neg, mag }
    }

    pub fn zero() -> Self {
        Grande::default()
    }

    pub fn de_i64(x: i64) -> Self {
        let u = x.unsigned_abs();
        Grande::novo(x < 0, vec![u as u32, (u >> 32) as u32])
    }

    pub fn e_zero(&self) -> bool {
        self.mag.is_empty()
    }

    /// Número de bits da magnitude.
    pub fn bits(&self) -> u64 {
        match self.mag.last() {
            None => 0,
            Some(&t) => 32 * self.mag.len() as u64 - u64::from(t.leading_zeros()),
        }
    }

    /// O valor como `i64`, se couber.
    pub fn para_i64(&self) -> Option<i64> {
        if self.mag.len() > 2 {
            return None;
        }
        let u = u64::from(self.mag.first().copied().unwrap_or(0)) | (u64::from(self.mag.get(1).copied().unwrap_or(0)) << 32);
        if self.neg {
            (u <= 1 << 63).then(|| (u as i64).wrapping_neg())
        } else {
            i64::try_from(u).ok()
        }
    }

    /// NumberToBigInt: só inteiros finitos.
    pub fn de_f64(x: f64) -> Option<Self> {
        if !x.is_finite() || x.fract() != 0.0 {
            return None;
        }
        let b = x.abs().to_bits();
        let exp = ((b >> 52) & 0x7FF) as i64;
        if exp == 0 {
            return Some(Grande::zero()); // só o zero (subnormais não são inteiros)
        }
        let mantissa = (b & ((1 << 52) - 1)) | (1 << 52);
        let k = exp - 1075; // x = mantissa * 2^k
        let m = Grande::novo(false, vec![mantissa as u32, (mantissa >> 32) as u32]);
        let mag = if k >= 0 { desl_esq_mag(&m.mag, k as u64) } else { desl_dir_mag(&m.mag, (-k) as u64).0 };
        Some(Grande::novo(x < 0.0, mag))
    }

    /// Para `f64`, arredondando ao par mais próximo (Number(x)).
    pub fn para_f64(&self) -> f64 {
        let n = self.bits();
        let v = if n <= 64 {
            let u = u64::from(self.mag.first().copied().unwrap_or(0)) | (u64::from(self.mag.get(1).copied().unwrap_or(0)) << 32);
            u as f64
        } else {
            // 64 bits do topo + um bit "algo caiu": o arredondamento do `u64 as f64` fica certo.
            let (topo, perdeu) = desl_dir_mag(&self.mag, n - 64);
            let u = (u64::from(topo[0]) | (u64::from(topo[1]) << 32)) | u64::from(perdeu);
            if n - 64 > 1100 {
                f64::INFINITY
            } else {
                u as f64 * 2f64.powi((n - 64) as i32)
            }
        };
        if self.neg {
            -v
        } else {
            v
        }
    }

    /// Dígitos ASCII numa base (sem sinal nem prefixo); `None` se algum não vale.
    pub fn de_digitos(digitos: &[u8], base: u32) -> Option<Self> {
        let mut mag: Vec<u32> = Vec::new();
        for &c in digitos {
            let d = (c as char).to_digit(base)?;
            let mut vai = u64::from(d);
            for x in mag.iter_mut() {
                let t = u64::from(*x) * u64::from(base) + vai;
                *x = t as u32;
                vai = t >> 32;
            }
            if vai != 0 {
                mag.push(vai as u32);
            }
        }
        Some(Grande::novo(false, mag))
    }

    /// Texto na base (2 a 36), com `-` se negativo.
    pub fn para_texto(&self, base: u32) -> String {
        if self.e_zero() {
            return "0".into();
        }
        // Divide pelo maior pedaço base^k que cabe numa palavra.
        let (mut pedaco, mut k) = (base, 1);
        while u64::from(pedaco) * u64::from(base) <= u64::from(u32::MAX) {
            pedaco *= base;
            k += 1;
        }
        let mut digitos = Vec::new();
        let mut a = self.mag.clone();
        while !a.is_empty() {
            let (q, mut r) = dividir_palavra(&a, pedaco);
            for _ in 0..k {
                digitos.push(char::from_digit(r % base, base).unwrap_or('0'));
                r /= base;
                if q.is_empty() && r == 0 {
                    break;
                }
            }
            a = q;
        }
        if self.neg {
            digitos.push('-');
        }
        digitos.iter().rev().collect()
    }

    pub fn negar(&self) -> Self {
        Grande::novo(!self.neg, self.mag.clone())
    }

    pub fn cmp(&self, o: &Grande) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &o.mag),
            (true, true) => cmp_mag(&o.mag, &self.mag),
        }
    }

    /// Comparação exata com um `f64`; `None` se ele é NaN.
    pub fn cmp_f64(&self, y: f64) -> Option<Ordering> {
        if y.is_nan() {
            return None;
        }
        if y.is_infinite() {
            return Some(if y > 0.0 { Ordering::Less } else { Ordering::Greater });
        }
        let piso = y.floor();
        let o = self.cmp(&Grande::de_f64(piso)?);
        // y fracionário: x == piso quer dizer x < y.
        Some(if o == Ordering::Equal && piso != y { Ordering::Less } else { o })
    }

    pub fn somar(&self, o: &Grande) -> Self {
        if self.neg == o.neg {
            return Grande::novo(self.neg, somar_mag(&self.mag, &o.mag));
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Less => Grande::novo(o.neg, subtrair_mag(&o.mag, &self.mag)),
            _ => Grande::novo(self.neg, subtrair_mag(&self.mag, &o.mag)),
        }
    }

    pub fn subtrair(&self, o: &Grande) -> Self {
        self.somar(&o.negar())
    }

    pub fn multiplicar(&self, o: &Grande) -> Self {
        Grande::novo(self.neg != o.neg, multiplicar_mag(&self.mag, &o.mag))
    }

    /// Divisão que trunca para o zero: (quociente, resto com o sinal do dividendo).
    /// `None` na divisão por zero.
    pub fn dividir(&self, o: &Grande) -> Option<(Self, Self)> {
        if o.e_zero() {
            return None;
        }
        let (q, r) = dividir_mag(&self.mag, &o.mag);
        Some((Grande::novo(self.neg != o.neg, q), Grande::novo(self.neg, r)))
    }

    /// `self ** e`; `Err` com a mensagem do RangeError.
    pub fn potencia(&self, e: &Grande) -> Result<Self, &'static str> {
        if e.neg {
            return Err("Exponent must be non-negative");
        }
        if e.e_zero() {
            return Ok(Grande::de_i64(1));
        }
        if self.e_zero() || (self.mag == [1]) {
            let impar = e.mag[0] & 1 == 1;
            return Ok(Grande::novo(self.neg && impar, self.mag.clone()));
        }
        let Some(n) = e.para_i64().filter(|&n| (n as u64).saturating_mul(self.bits() - 1) < MAX_BITS) else {
            return Err("Maximum BigInt size exceeded");
        };
        let (mut r, mut b, mut n) = (Grande::de_i64(1), self.clone(), n as u64);
        while n > 0 {
            if n & 1 == 1 {
                r = r.multiplicar(&b);
            }
            n >>= 1;
            if n > 0 {
                b = b.multiplicar(&b);
            }
        }
        Ok(r)
    }

    /// `self << k` (k negativo desloca para a direita, arredondando para baixo).
    pub fn deslocar(&self, k: &Grande) -> Result<Self, &'static str> {
        if k.neg {
            let n = k.para_i64().map_or(u64::MAX, |n| n.unsigned_abs());
            let (mag, perdeu) = desl_dir_mag(&self.mag, n);
            let r = Grande::novo(self.neg, mag);
            // Negativo que perdeu bits: o piso é um a menos.
            return Ok(if self.neg && perdeu { r.subtrair(&Grande::de_i64(1)) } else { r });
        }
        if self.e_zero() {
            return Ok(Grande::zero());
        }
        match k.para_i64() {
            Some(n) if (n as u64) + self.bits() <= MAX_BITS => Ok(Grande::novo(self.neg, desl_esq_mag(&self.mag, n as u64))),
            _ => Err("Maximum BigInt size exceeded"),
        }
    }

    /// Complemento de dois em `n` palavras.
    fn comp2(&self, n: usize) -> Vec<u32> {
        let mut w = self.mag.clone();
        w.resize(n, 0);
        if self.neg {
            let mut vai = 1u64;
            for x in w.iter_mut() {
                let t = u64::from(!*x) + vai;
                *x = t as u32;
                vai = t >> 32;
            }
        }
        w
    }

    fn de_comp2(mut w: Vec<u32>) -> Self {
        let neg = w.last().is_some_and(|&t| t >> 31 == 1);
        if neg {
            let mut vai = 1u64;
            for x in w.iter_mut() {
                let t = u64::from(!*x) + vai;
                *x = t as u32;
                vai = t >> 32;
            }
        }
        Grande::novo(neg, w)
    }

    /// `&`, `|` e `^` pelo complemento de dois (o sinal se estende ao infinito).
    pub fn bit_a_bit(&self, o: &Grande, f: impl Fn(u32, u32) -> u32) -> Self {
        let n = self.mag.len().max(o.mag.len()) + 1;
        let (a, b) = (self.comp2(n), o.comp2(n));
        Grande::de_comp2(a.iter().zip(&b).map(|(&x, &y)| f(x, y)).collect())
    }

    /// `~x` = `-x - 1`.
    pub fn nao(&self) -> Self {
        self.negar().subtrair(&Grande::de_i64(1))
    }

    /// `BigInt.asUintN` / `BigInt.asIntN`.
    pub fn como_n_bits(&self, bits: u64, com_sinal: bool) -> Result<Self, &'static str> {
        if bits == 0 {
            return Ok(Grande::zero());
        }
        if bits > self.bits() + 1 {
            // Cabe inteiro: só o negativo sem sinal muda (vira 2^bits + x).
            if com_sinal || !self.neg {
                return Ok(self.clone());
            }
            if bits > MAX_BITS {
                return Err("Maximum BigInt size exceeded");
            }
            return Ok(Grande::de_i64(1).deslocar(&Grande::de_i64(bits as i64))?.somar(self));
        }
        let palavras = bits.div_ceil(32) as usize;
        let mut w = self.comp2(self.mag.len() + 1);
        w.truncate(palavras);
        let sobra = (palavras as u64 * 32 - bits) as u32;
        if sobra > 0 {
            let t = w.len() - 1;
            w[t] &= u32::MAX >> sobra;
        }
        let u = Grande::novo(false, w);
        let alto = (bits - 1) as usize;
        if com_sinal && u.mag.get(alto / 32).is_some_and(|&x| x >> (alto % 32) & 1 == 1) {
            return Ok(u.subtrair(&Grande::de_i64(1).deslocar(&Grande::de_i64(bits as i64))?));
        }
        Ok(u)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn g(s: &str) -> Grande {
        let (neg, d) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
        let x = Grande::de_digitos(d.as_bytes(), 10).unwrap();
        if neg {
            x.negar()
        } else {
            x
        }
    }

    #[test]
    fn texto_ida_e_volta() {
        for s in ["0", "1", "-1", "4294967295", "4294967296", "-18446744073709551617", "123456789012345678901234567890"] {
            assert_eq!(g(s).para_texto(10), s);
        }
        assert_eq!(g("255").para_texto(16), "ff");
        assert_eq!(g("-5").para_texto(2), "-101");
    }

    #[test]
    fn contas() {
        let a = g("123456789012345678901234567890");
        let b = g("987654321098765432109876543210");
        assert_eq!(a.multiplicar(&b).para_texto(10), "121932631137021795226185032733622923332237463801111263526900");
        let (q, r) = b.dividir(&a).unwrap();
        assert_eq!((q.para_texto(10).as_str(), r.para_texto(10).as_str()), ("8", "9000000000900000000090"));
        let (q, r) = g("-7").dividir(&g("2")).unwrap();
        assert_eq!((q.para_texto(10).as_str(), r.para_texto(10).as_str()), ("-3", "-1"));
        // Divisão longa com várias palavras no divisor (confere q*b + r == a).
        let x = g("340282366920938463463374607431768211455123456789");
        let y = g("18446744073709551629");
        let (q, r) = x.dividir(&y).unwrap();
        assert_eq!(q.multiplicar(&y).somar(&r), x);
        assert_eq!(r.cmp(&y), Ordering::Less);
        assert_eq!(g("2").potencia(&g("100")).unwrap().para_texto(10), "1267650600228229401496703205376");
        assert_eq!(a.subtrair(&a), Grande::zero());
    }

    #[test]
    fn bits() {
        assert_eq!(g("-5").bit_a_bit(&g("3"), |x, y| x & y).para_texto(10), "3");
        assert_eq!(g("-5").bit_a_bit(&g("3"), |x, y| x | y).para_texto(10), "-5");
        assert_eq!(g("-5").bit_a_bit(&g("3"), |x, y| x ^ y).para_texto(10), "-8");
        assert_eq!(g("-5").deslocar(&g("-1")).unwrap().para_texto(10), "-3");
        assert_eq!(g("1").deslocar(&g("64")).unwrap().para_texto(10), "18446744073709551616");
        assert_eq!(g("255").como_n_bits(8, true).unwrap().para_texto(10), "-1");
        assert_eq!(g("-1").como_n_bits(64, false).unwrap().para_texto(10), "18446744073709551615");
        assert_eq!(g("-1").como_n_bits(8, false).unwrap().para_texto(10), "255");
    }

    #[test]
    fn f64() {
        assert_eq!(Grande::de_f64(1e20).unwrap().para_texto(10), "100000000000000000000");
        assert_eq!(Grande::de_f64(-3.0).unwrap().para_texto(10), "-3");
        assert!(Grande::de_f64(1.5).is_none());
        assert_eq!(g("9007199254740993").para_f64(), 9007199254740992.0);
        assert_eq!(g("123456789012345678901234567890").para_f64(), 1.2345678901234568e29);
        assert_eq!(g("2").cmp_f64(2.5), Some(Ordering::Less));
        assert_eq!(g("-3").cmp_f64(-2.5), Some(Ordering::Less));
    }
}
