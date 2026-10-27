//! Conversões numéricas do Falcão, seguindo o texto da ECMA-262.
//!
//! Funções puras sobre f64 e fatias UTF-16. A aritmética exata usa um inteiro
//! grande simples (`Grande`, base 2^32) escrito aqui mesmo, sem dependências.

use std::cmp::Ordering;

const ALFABETO: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Limite de dígitos na parte fracionária de `toString(raiz)`.
const LIMITE_DIGITOS: usize = 1100;

// ---------------------------------------------------------------- inteiro grande

/// Inteiro sem sinal em base 2^32, little-endian. Vetor vazio significa zero.
/// Todas as operações mantêm o vetor normalizado (sem zeros à esquerda).
#[derive(Clone, Debug)]
struct Grande {
    l: Vec<u32>,
}

impl Grande {
    fn de_u64(v: u64) -> Self {
        let mut g = Grande { l: vec![v as u32, (v >> 32) as u32] };
        g.normaliza();
        g
    }

    fn normaliza(&mut self) {
        while self.l.last() == Some(&0) {
            self.l.pop();
        }
    }

    fn e_zero(&self) -> bool {
        self.l.is_empty()
    }

    /// Os 64 bits baixos.
    fn valor_u64(&self) -> u64 {
        let lo = self.l.first().copied().unwrap_or(0) as u64;
        let hi = self.l.get(1).copied().unwrap_or(0) as u64;
        (hi << 32) | lo
    }

    /// Valor quando ele cabe em 32 bits (usado para dígitos de base pequena).
    fn como_u32(&self) -> u32 {
        self.l.first().copied().unwrap_or(0)
    }

    /// self *= m
    fn mul_pequeno(&mut self, m: u32) {
        let mut transporte: u64 = 0;
        for x in self.l.iter_mut() {
            let v = (*x as u64) * (m as u64) + transporte;
            *x = v as u32;
            transporte = v >> 32;
        }
        if transporte > 0 {
            self.l.push(transporte as u32);
        }
        self.normaliza();
    }

    /// self += a
    fn soma_pequena(&mut self, a: u32) {
        let mut transporte = a as u64;
        for x in self.l.iter_mut() {
            if transporte == 0 {
                break;
            }
            let v = *x as u64 + transporte;
            *x = v as u32;
            transporte = v >> 32;
        }
        if transporte > 0 {
            self.l.push(transporte as u32);
        }
    }

    /// self /= d; devolve o resto.
    fn div_pequeno(&mut self, d: u32) -> u32 {
        let mut resto: u64 = 0;
        for x in self.l.iter_mut().rev() {
            let cur = (resto << 32) | (*x as u64);
            *x = (cur / d as u64) as u32;
            resto = cur % d as u64;
        }
        self.normaliza();
        resto as u32
    }

    /// self << n
    fn shl(&self, n: usize) -> Self {
        let (pal, bits) = (n / 32, n % 32);
        let mut l = vec![0u32; pal];
        let mut transporte = 0u32;
        for &x in &self.l {
            if bits == 0 {
                l.push(x);
            } else {
                l.push((x << bits) | transporte);
                transporte = x >> (32 - bits);
            }
        }
        if transporte != 0 {
            l.push(transporte);
        }
        let mut g = Grande { l };
        g.normaliza();
        g
    }

    /// self >> n
    fn shr(&self, n: usize) -> Self {
        let (pal, bits) = (n / 32, n % 32);
        if pal >= self.l.len() {
            return Grande { l: Vec::new() };
        }
        let src = &self.l[pal..];
        let mut l = Vec::with_capacity(src.len());
        for i in 0..src.len() {
            let baixo = src[i] >> bits;
            let alto = if bits > 0 {
                src.get(i + 1).map_or(0, |&x| x << (32 - bits))
            } else {
                0
            };
            l.push(baixo | alto);
        }
        let mut g = Grande { l };
        g.normaliza();
        g
    }

    /// Mantém só os n bits baixos (self mod 2^n).
    fn bits_baixos(&self, n: usize) -> Self {
        let (pal, bits) = (n / 32, n % 32);
        let mut l: Vec<u32> = self.l.iter().take(pal).copied().collect();
        if bits > 0 {
            if let Some(&x) = self.l.get(pal) {
                l.push(x & ((1u32 << bits) - 1));
            }
        }
        let mut g = Grande { l };
        g.normaliza();
        g
    }

    fn bit_len(&self) -> usize {
        match self.l.last() {
            None => 0,
            Some(&t) => (self.l.len() - 1) * 32 + (32 - t.leading_zeros() as usize),
        }
    }

    fn bit(&self, i: usize) -> bool {
        self.l.get(i / 32).is_some_and(|&x| (x >> (i % 32)) & 1 == 1)
    }

    fn somar(&self, o: &Grande) -> Grande {
        let n = self.l.len().max(o.l.len());
        let mut l = Vec::with_capacity(n + 1);
        let mut transporte = 0u64;
        for i in 0..n {
            let a = self.l.get(i).copied().unwrap_or(0) as u64;
            let b = o.l.get(i).copied().unwrap_or(0) as u64;
            let v = a + b + transporte;
            l.push(v as u32);
            transporte = v >> 32;
        }
        if transporte > 0 {
            l.push(transporte as u32);
        }
        let mut g = Grande { l };
        g.normaliza();
        g
    }

    fn comparar(&self, o: &Grande) -> Ordering {
        if self.l.len() != o.l.len() {
            return self.l.len().cmp(&o.l.len());
        }
        for i in (0..self.l.len()).rev() {
            match self.l[i].cmp(&o.l[i]) {
                Ordering::Equal => {}
                outro => return outro,
            }
        }
        Ordering::Equal
    }

    /// Dígitos decimais ASCII, sem zeros à esquerda ("0" para zero).
    fn para_decimal(&self) -> Vec<u8> {
        if self.e_zero() {
            return vec![b'0'];
        }
        let mut g = self.clone();
        let mut blocos: Vec<u32> = Vec::new();
        while !g.e_zero() {
            blocos.push(g.div_pequeno(1_000_000_000));
        }
        let mut out = Vec::new();
        for (i, &b) in blocos.iter().rev().enumerate() {
            // o bloco mais significativo não leva zeros à esquerda
            let txt = if i == 0 { format!("{}", b) } else { format!("{:09}", b) };
            out.extend_from_slice(txt.as_bytes());
        }
        out
    }

    /// Dígitos na base dada, do mais significativo ao menos (zero vira [0]).
    fn digitos_na_base(&self, base: u32) -> Vec<u32> {
        if self.e_zero() {
            return vec![0];
        }
        let mut g = self.clone();
        let mut d = Vec::new();
        while !g.e_zero() {
            d.push(g.div_pequeno(base));
        }
        d.reverse();
        d
    }

    /// Converte para f64 com arredondamento correto (empate para o par).
    fn para_f64(&self) -> f64 {
        let bl = self.bit_len();
        if bl <= 64 {
            return self.valor_u64() as f64; // conversão u64 -> f64 já arredonda ao par
        }
        let sh = bl - 53;
        let mut m = self.shr(sh).valor_u64();
        let arredonda = self.bit(sh - 1);
        let resto = !self.bits_baixos(sh - 1).e_zero();
        if arredonda && (resto || m & 1 == 1) {
            m += 1;
        }
        (m as f64) * 2f64.powi(sh as i32)
    }
}

// ---------------------------------------------------------------- partes do f64

/// Decompõe um f64 positivo finito em `m * 2^e` (m inteiro, e inteiro).
fn decompor(x: f64) -> (u64, i32) {
    let bits = x.to_bits();
    let expo = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    if expo == 0 {
        (frac, -1074) // subnormal
    } else {
        (frac | (1u64 << 52), expo - 1075)
    }
}

/// Dígitos mais curtos que voltam ao mesmo f64 (via `{:e}` da biblioteca padrão).
/// Devolve (dígitos, n) com valor = 0.dígitos * 10^n. Só para x > 0 finito.
fn digitos_curtos(a: f64) -> (String, i32) {
    let txt = format!("{:e}", a);
    let (mant, exp) = txt.split_once('e').unwrap_or((txt.as_str(), "0"));
    let mut d: String = mant.chars().filter(|&c| c != '.').collect();
    while d.len() > 1 && d.ends_with('0') {
        d.pop();
    }
    let n = exp.parse::<i32>().unwrap_or(0) + 1;
    (d, n)
}

/// Sufixo de expoente "e+N" / "e-N".
fn expoente(e: i64) -> String {
    format!("e{}{}", if e < 0 { '-' } else { '+' }, e.abs())
}

fn texto(d: &[u8]) -> String {
    d.iter().map(|&c| c as char).collect()
}

/// Decimal exato de um f64 positivo (ou zero). Devolve (dígitos, ponto) com
/// valor = 0.dígitos * 10^ponto. Para x = m/2^k, usa m*5^k / 10^k.
fn decimal_exato(a: f64) -> (Vec<u8>, i64) {
    if a == 0.0 {
        return (vec![b'0'], 1);
    }
    let (m, e) = decompor(a);
    if e >= 0 {
        let s = Grande::de_u64(m).shl(e as usize).para_decimal();
        let n = s.len() as i64;
        (s, n)
    } else {
        let k = (-e) as usize;
        let mut g = Grande::de_u64(m);
        for _ in 0..k {
            g.mul_pequeno(5);
        }
        let s = g.para_decimal();
        let n = s.len() as i64 - k as i64;
        (s, n)
    }
}

/// Arredonda o valor (0.s * 10^j) para inteiro, metade para cima (escolhe o maior
/// n em caso de empate). Devolve exatamente `j` dígitos, ou `j + 1` se houver
/// transbordo (ex.: 99 -> 100). Com j <= 0 o resultado é vazio ou "1".
fn arredonda_inteiro(s: &[u8], j: i64) -> Vec<u8> {
    let mut d: Vec<u8> = (0..(j.max(0) as usize))
        .map(|i| s.get(i).copied().unwrap_or(b'0'))
        .collect();
    let rodada = if j < 0 { b'0' } else { s.get(j as usize).copied().unwrap_or(b'0') };
    if rodada >= b'5' {
        let mut i = d.len();
        let mut sobe = true;
        while sobe && i > 0 {
            i -= 1;
            if d[i] == b'9' {
                d[i] = b'0';
            } else {
                d[i] += 1;
                sobe = false;
            }
        }
        if sobe {
            d.insert(0, b'1');
        }
    }
    d
}

/// `p` dígitos significativos de um f64 positivo, com o expoente do primeiro dígito.
fn digitos_significativos(a: f64, p: usize) -> (Vec<u8>, i64) {
    let (s, ponto) = decimal_exato(a);
    let mut d = arredonda_inteiro(&s, p as i64);
    let mut e = ponto - 1;
    if d.len() > p {
        d.pop(); // transbordo: era exatamente 10^p
        e += 1;
    }
    (d, e)
}

// ---------------------------------------------------------------- Number::toString

/// Number::toString(x) com raiz 10 (ECMA-262 §6.1.6.1.20).
pub fn numero_para_texto(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x == 0.0 {
        return "0".to_string(); // cobre +0 e -0
    }
    if x.is_infinite() {
        return String::from(if x > 0.0 { "Infinity" } else { "-Infinity" });
    }
    let sinal = if x < 0.0 { "-" } else { "" };
    let (d, n) = digitos_curtos(x.abs());
    let k = d.len() as i32;
    let corpo = if k <= n && n <= 21 {
        format!("{}{}", d, "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &d[..n as usize], &d[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{}", "0".repeat((-n) as usize), d)
    } else {
        let mant = if k == 1 { d.clone() } else { format!("{}.{}", &d[..1], &d[1..]) };
        format!("{}{}", mant, expoente((n - 1) as i64))
    };
    format!("{}{}", sinal, corpo)
}

/// Dígitos da parte fracionária de m/2^k na base `base`, com a regra de parada
/// de `toString(raiz)`: para quando o resto fica menor que metade da unidade do
/// último bit do f64 (ou quando o resto + essa metade já passa de 1).
/// Devolve (dígitos, subir): `subir` indica arredondar o último dígito para cima.
fn fracao_na_base(m: u64, k: usize, base: u32) -> (Vec<u32>, bool) {
    // valor = F / 2^k, com F = m mod 2^k. A meia unidade do f64 é 1/2^(k+1).
    // Trabalhamos com o dobro: numerador R sobre 2^(k+1) e meia unidade = 1.
    let mut r = Grande::de_u64(m).bits_baixos(k).shl(1);
    let mut digitos = Vec::new();
    if r.e_zero() {
        return (digitos, false);
    }
    let d_bits = k + 1;
    let total = Grande::de_u64(1).shl(d_bits);
    let mut meia = Grande::de_u64(1);
    while digitos.len() < LIMITE_DIGITOS {
        r.mul_pequeno(base);
        digitos.push(r.shr(d_bits).como_u32());
        r = r.bits_baixos(d_bits);
        meia.mul_pequeno(base);
        let baixo = r.comparar(&meia) == Ordering::Less;
        let alto = r.somar(&meia).comparar(&total) == Ordering::Greater;
        if baixo || alto {
            let subir = alto && (!baixo || r.shl(1).comparar(&total) != Ordering::Less);
            return (digitos, subir);
        }
    }
    (digitos, false)
}

/// Soma 1 nos dígitos (base `base`); devolve true se transbordou tudo.
fn incrementa(d: &mut [u32], base: u32) -> bool {
    for x in d.iter_mut().rev() {
        if *x + 1 < base {
            *x += 1;
            return false;
        }
        *x = 0;
    }
    true
}

/// Number.prototype.toString(raiz). Para raiz 10 chama `numero_para_texto`.
/// Raiz fora de 2..=36 cai em base 10: quem chama deve validar (RangeError no JS).
pub fn numero_para_texto_raiz(x: f64, raiz: u32) -> String {
    if raiz == 10 || !(2..=36).contains(&raiz) || !x.is_finite() || x == 0.0 {
        return numero_para_texto(x);
    }
    let sinal = if x < 0.0 { "-" } else { "" };
    let (m, e) = decompor(x.abs());
    let inteira = if e >= 0 {
        Grande::de_u64(m).shl(e as usize)
    } else {
        Grande::de_u64(m).shr((-e) as usize)
    };
    let mut ip = inteira.digitos_na_base(raiz);
    let (mut fr, subir) = if e < 0 {
        fracao_na_base(m, (-e) as usize, raiz)
    } else {
        (Vec::new(), false)
    };
    if subir && incrementa(&mut fr, raiz) && incrementa(&mut ip, raiz) {
        ip.insert(0, 1);
    }
    while fr.last() == Some(&0) {
        fr.pop();
    }
    let mut s = String::from(sinal);
    s.extend(ip.iter().map(|&d| ALFABETO[d as usize] as char));
    if !fr.is_empty() {
        s.push('.');
        s.extend(fr.iter().map(|&d| ALFABETO[d as usize] as char));
    }
    s
}

// ---------------------------------------------------------------- toFixed, toExponential, toPrecision

/// Number.prototype.toFixed(casas). Para |x| >= 1e21 devolve `numero_para_texto`.
/// Arredondamento exato do valor binário; empate vai para o maior n.
pub fn para_fixo(x: f64, casas: u32) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return numero_para_texto(x);
    }
    let sinal = if x < 0.0 { "-" } else { "" };
    let f = casas as usize;
    let (s, ponto) = decimal_exato(x.abs());
    let mut d = arredonda_inteiro(&s, ponto + casas as i64);
    while d.len() < f + 1 {
        d.insert(0, b'0');
    }
    if f == 0 {
        return format!("{}{}", sinal, texto(&d));
    }
    let corte = d.len() - f;
    format!("{}{}.{}", sinal, texto(&d[..corte]), texto(&d[corte..]))
}

/// Number.prototype.toExponential(casas). `None` usa os dígitos mais curtos.
pub fn para_exponencial(x: f64, casas: Option<u32>) -> String {
    if !x.is_finite() {
        return numero_para_texto(x);
    }
    let sinal = if x < 0.0 { "-" } else { "" };
    let a = x.abs();
    let (d, e): (String, i64) = if a == 0.0 {
        ("0".repeat(casas.unwrap_or(0) as usize + 1), 0)
    } else if let Some(f) = casas {
        let (v, e) = digitos_significativos(a, f as usize + 1);
        (texto(&v), e)
    } else {
        let (s, n) = digitos_curtos(a);
        (s, n as i64 - 1)
    };
    let mut out = format!("{}{}", sinal, &d[..1]);
    if d.len() > 1 {
        out.push('.');
        out.push_str(&d[1..]);
    }
    out.push_str(&expoente(e));
    out
}

/// Number.prototype.toPrecision(p). `p` é limitado a 1..=100 (quem chama deve validar).
pub fn para_precisao(x: f64, p: u32) -> String {
    if !x.is_finite() {
        return numero_para_texto(x);
    }
    let sinal = if x < 0.0 { "-" } else { "" };
    let a = x.abs();
    let p = p.clamp(1, 100) as usize;
    let (d, e): (String, i64) = if a == 0.0 {
        ("0".repeat(p), 0)
    } else {
        let (v, e) = digitos_significativos(a, p);
        (texto(&v), e)
    };
    let corpo = if e < -6 || e >= p as i64 {
        let mut c = d[..1].to_string();
        if p != 1 {
            c.push('.');
            c.push_str(&d[1..]);
        }
        c.push_str(&expoente(e));
        c
    } else if e == p as i64 - 1 {
        d
    } else if e >= 0 {
        let k = e as usize + 1;
        format!("{}.{}", &d[..k], &d[k..])
    } else {
        format!("0.{}{}", "0".repeat((-(e + 1)) as usize), d)
    };
    format!("{}{}", sinal, corpo)
}

// ---------------------------------------------------------------- texto -> número

/// WhiteSpace e LineTerminator da ECMA-262 (§12.2 e §12.3).
pub(crate) fn e_espaco(c: u16) -> bool {
    matches!(
        c,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

fn tira_inicio(t: &[u16]) -> &[u16] {
    let ini = t.iter().position(|&c| !e_espaco(c)).unwrap_or(t.len());
    &t[ini..]
}

fn tira_espacos(t: &[u16]) -> &[u16] {
    let t = tira_inicio(t);
    let fim = t.iter().rposition(|&c| !e_espaco(c)).map_or(0, |p| p + 1);
    &t[..fim]
}

/// Prefixo ASCII da fatia (para as funções de prefixo, que só aceitam ASCII).
fn prefixo_ascii(t: &[u16]) -> Vec<u8> {
    t.iter().take_while(|&&c| c < 0x80).map(|&c| c as u8).collect()
}

/// Valida StrUnsignedDecimalLiteral com sinal opcional (sem "Infinity").
fn decimal_valido(s: &[u8]) -> bool {
    let n = s.len();
    let mut i = 0;
    if i < n && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let ini = i;
    while i < n && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut digs = i - ini;
    if i < n && s[i] == b'.' {
        i += 1;
        let f = i;
        while i < n && s[i].is_ascii_digit() {
            i += 1;
        }
        digs += i - f;
    }
    if digs == 0 {
        return false;
    }
    if i < n && (s[i] == b'e' || s[i] == b'E') {
        i += 1;
        if i < n && (s[i] == b'+' || s[i] == b'-') {
            i += 1;
        }
        let ef = i;
        while i < n && s[i].is_ascii_digit() {
            i += 1;
        }
        if i == ef {
            return false;
        }
    }
    i == n
}

/// Inteiro em base 2, 8, 16 ou 32 com dígitos já validados, arredondado para f64.
fn inteiro_na_base(d: &[u8], base: u32) -> f64 {
    let mut g = Grande::de_u64(0);
    for &c in d {
        g.mul_pequeno(base);
        g.soma_pequena((c as char).to_digit(base).unwrap_or(0));
    }
    g.para_f64()
}

/// StringToNumber (ECMA-262 §7.1.4.1.1).
pub fn texto_para_numero(t: &[u16]) -> f64 {
    let t = tira_espacos(t);
    if t.is_empty() {
        return 0.0;
    }
    if t.iter().any(|&c| c > 0x7f) {
        return f64::NAN;
    }
    let s: Vec<u8> = t.iter().map(|&c| c as u8).collect();
    if s.len() >= 2 && s[0] == b'0' {
        let base = match s[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if base != 0 {
            // hexa, octal e binário não aceitam sinal
            let corpo = &s[2..];
            if corpo.is_empty() || corpo.iter().any(|&c| (c as char).to_digit(base).is_none()) {
                return f64::NAN;
            }
            return inteiro_na_base(corpo, base);
        }
    }
    let (negativo, corpo) = match s[0] {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, &s[..]),
    };
    if corpo == &b"Infinity"[..] {
        return if negativo { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    if !decimal_valido(corpo) {
        return f64::NAN;
    }
    // a validação acima já garante a forma; o parse da biblioteca arredonda certo
    std::str::from_utf8(&s).ok().and_then(|x| x.parse::<f64>().ok()).unwrap_or(f64::NAN)
}

/// ToInt32 (§7.1.6). NaN e infinitos viram 0.
pub fn para_int32(x: f64) -> i32 {
    para_uint32(x) as i32
}

/// ToUint32 (§7.1.7): trunca e reduz módulo 2^32.
pub fn para_uint32(x: f64) -> u32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc() % 4294967296.0; // fmod é exato
    let m = if m < 0.0 { m + 4294967296.0 } else { m };
    m as u32
}

/// parseFloat (ECMA-262 §19.2.4): maior prefixo válido de StrDecimalLiteral.
pub fn parse_float(t: &[u16]) -> f64 {
    let s = prefixo_ascii(tira_inicio(t));
    let n = s.len();
    let mut i = 0;
    let mut negativo = false;
    if i < n && (s[i] == b'+' || s[i] == b'-') {
        negativo = s[i] == b'-';
        i += 1;
    }
    if s[i..].starts_with(&b"Infinity"[..]) {
        return if negativo { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    let mut digs = 0;
    while i < n && s[i].is_ascii_digit() {
        i += 1;
        digs += 1;
    }
    if i < n && s[i] == b'.' {
        let mut k = i + 1;
        let mut fd = 0;
        while k < n && s[k].is_ascii_digit() {
            k += 1;
            fd += 1;
        }
        if digs + fd > 0 {
            i = k;
            digs += fd;
        }
    }
    if digs == 0 {
        return f64::NAN;
    }
    if i < n && (s[i] == b'e' || s[i] == b'E') {
        let mut k = i + 1;
        if k < n && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        let ek = k;
        while k < n && s[k].is_ascii_digit() {
            k += 1;
        }
        if k > ek {
            i = k; // expoente sem dígitos não entra no prefixo
        }
    }
    std::str::from_utf8(&s[..i]).ok().and_then(|x| x.parse::<f64>().ok()).unwrap_or(f64::NAN)
}

/// parseInt (ECMA-262 §19.2.5).
pub fn parse_int(t: &[u16], raiz: i32) -> f64 {
    let s = prefixo_ascii(tira_inicio(t));
    let mut i = 0;
    let mut sinal = 1.0;
    if s.first() == Some(&b'-') {
        sinal = -1.0;
        i = 1;
    } else if s.first() == Some(&b'+') {
        i = 1;
    }
    let mut base: u32 = 10;
    let mut tira_prefixo = true;
    if raiz != 0 {
        if !(2..=36).contains(&raiz) {
            return f64::NAN;
        }
        base = raiz as u32;
        tira_prefixo = raiz == 16;
    }
    if tira_prefixo && s.len() >= i + 2 && s[i] == b'0' && (s[i + 1] == b'x' || s[i + 1] == b'X') {
        i += 2;
        base = 16;
    }
    let ini = i;
    while i < s.len() && (s[i] as char).to_digit(base).is_some() {
        i += 1;
    }
    let dig = &s[ini..i];
    if dig.is_empty() {
        return f64::NAN;
    }
    let valor = if base == 10 {
        std::str::from_utf8(dig).ok().and_then(|x| x.parse::<f64>().ok()).unwrap_or(f64::NAN)
    } else {
        inteiro_na_base(dig, base)
    };
    sinal * valor
}

// ---------------------------------------------------------------- testes

#[cfg(test)]
mod testes {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn numero_para_texto_exemplos() {
        assert_eq!(numero_para_texto(f64::NAN), "NaN");
        assert_eq!(numero_para_texto(0.0), "0");
        assert_eq!(numero_para_texto(-0.0), "0");
        assert_eq!(numero_para_texto(f64::INFINITY), "Infinity");
        assert_eq!(numero_para_texto(f64::NEG_INFINITY), "-Infinity");
        assert_eq!(numero_para_texto(1e21), "1e+21");
        assert_eq!(numero_para_texto(1e20), "100000000000000000000");
        assert_eq!(numero_para_texto(0.000001), "0.000001");
        assert_eq!(numero_para_texto(1e-7), "1e-7");
        assert_eq!(numero_para_texto(123.456), "123.456");
        assert_eq!(numero_para_texto(-5.5), "-5.5");
        assert_eq!(numero_para_texto(9007199254740992.0), "9007199254740992");
        assert_eq!(numero_para_texto(0.1 + 0.2), "0.30000000000000004");
    }

    #[test]
    fn numero_para_texto_raiz_exemplos() {
        assert_eq!(numero_para_texto_raiz(0.5, 2), "0.1");
        assert_eq!(numero_para_texto_raiz(255.0, 16), "ff");
        assert_eq!(numero_para_texto_raiz(-255.0, 2), "-11111111");
        assert_eq!(numero_para_texto_raiz(35.0, 36), "z");
        assert_eq!(numero_para_texto_raiz(0.0, 2), "0");
    }

    #[test]
    fn para_fixo_exemplos() {
        assert_eq!(para_fixo(1.005, 2), "1.00");
        assert_eq!(para_fixo(1.45, 1), "1.4");
        assert_eq!(para_fixo(0.5, 0), "1");
        assert_eq!(para_fixo(-1.5, 0), "-2");
        assert_eq!(para_fixo(123.456, 10), "123.4560000000");
        assert_eq!(para_fixo(0.0, 2), "0.00");
        assert_eq!(para_fixo(1e21, 2), "1e+21");
    }

    #[test]
    fn para_exponencial_exemplos() {
        assert_eq!(para_exponencial(123456.0, Some(2)), "1.23e+5");
        assert_eq!(para_exponencial(0.00015, None), "1.5e-4");
    }

    #[test]
    fn para_precisao_exemplos() {
        assert_eq!(para_precisao(123.456, 4), "123.5");
        assert_eq!(para_precisao(0.000123, 2), "0.00012");
        assert_eq!(para_precisao(123456.0, 2), "1.2e+5");
        assert_eq!(para_precisao(9.9999, 2), "10");
    }

    #[test]
    fn texto_para_numero_exemplos() {
        assert_eq!(texto_para_numero(&u("")), 0.0);
        assert_eq!(texto_para_numero(&u("  42\n")), 42.0);
        assert_eq!(texto_para_numero(&u("\u{2028}\u{FEFF}7\u{3000}")), 7.0);
        assert_eq!(texto_para_numero(&u("Infinity")), f64::INFINITY);
        assert_eq!(texto_para_numero(&u("+Infinity")), f64::INFINITY);
        assert_eq!(texto_para_numero(&u("-Infinity")), f64::NEG_INFINITY);
        assert_eq!(texto_para_numero(&u(".5")), 0.5);
        assert_eq!(texto_para_numero(&u("5.")), 5.0);
        assert_eq!(texto_para_numero(&u("1e3")), 1000.0);
        let z = texto_para_numero(&u("-0"));
        assert!(z == 0.0 && z.is_sign_negative());
        assert_eq!(texto_para_numero(&u("0x1F")), 31.0);
        assert_eq!(texto_para_numero(&u("0o17")), 15.0);
        assert_eq!(texto_para_numero(&u("0b101")), 5.0);
        assert!(texto_para_numero(&u("0x")).is_nan());
        assert!(texto_para_numero(&u("-0x10")).is_nan());
        assert!(texto_para_numero(&u("_")).is_nan());
        assert!(texto_para_numero(&u("1n")).is_nan());
    }

    #[test]
    fn inteiros_de_32_bits() {
        assert_eq!(para_int32(4294967296.5), 0);
        assert_eq!(para_int32(2147483648.0), -2147483648);
        assert_eq!(para_int32(-1.0), -1);
        assert_eq!(para_uint32(-1.0), 4294967295);
        assert_eq!(para_int32(f64::NAN), 0);
    }

    #[test]
    fn parse_float_exemplos() {
        assert_eq!(parse_float(&u("3.14abc")), 3.14);
        assert!(parse_float(&u("abc")).is_nan());
        assert_eq!(parse_float(&u(".5e")), 0.5);
        assert_eq!(parse_float(&u("  -Infinityx")), f64::NEG_INFINITY);
        assert_eq!(parse_float(&u("-.5")), -0.5);
    }

    #[test]
    fn parse_int_exemplos() {
        assert_eq!(parse_int(&u("  0x1f"), 0), 31.0);
        assert_eq!(parse_int(&u("42px"), 10), 42.0);
        assert_eq!(parse_int(&u("-17"), 0), -17.0);
        assert_eq!(parse_int(&u("z"), 36), 35.0);
        assert_eq!(parse_int(&u("10"), 2), 2.0);
        assert_eq!(parse_int(&u("ff"), 16), 255.0);
        assert_eq!(parse_int(&u("0x1f"), 16), 31.0);
        assert!(parse_int(&u("abc"), 10).is_nan());
        assert!(parse_int(&u("123"), 1).is_nan());
        assert!(parse_int(&u("0x"), 16).is_nan());
    }
}
