//! Contas de data/hora do objeto `Date` do JavaScript (ECMAScript, seção 21.4).
//!
//! Tudo em Rust puro, só biblioteca padrão. O tempo é um `f64` com milissegundos
//! desde 1970-01-01T00:00:00Z; `NaN` significa data inválida.
//!
//! O fuso local vem da variável `TZ` (formatos "UTC", "UTC-3", "<-03>3" ou um
//! nome como "America/Sao_Paulo") ou, se nada disso der, de `/etc/localtime`
//! (arquivo TZif). A leitura é feita uma vez e guardada num `OnceLock`.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Milissegundos em um dia.
pub const MS_POR_DIA: f64 = 86_400_000.0;

const MS_POR_HORA: f64 = 3_600_000.0;
const MS_POR_MINUTO: f64 = 60_000.0;

const DIAS_ACUM: [f64; 12] = [
    0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0,
];
const DIAS_MES: [f64; 12] = [
    31.0, 28.0, 31.0, 30.0, 31.0, 30.0, 31.0, 31.0, 30.0, 31.0, 30.0, 31.0,
];
const NOMES_DIA: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const NOMES_MES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MESES_MIN: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DIAS_MIN: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

// ---------------------------------------------------------------------------
// Relógio
// ---------------------------------------------------------------------------

/// Instante atual em milissegundos desde 1970 UTC.
pub fn agora() -> f64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as f64,
        Err(e) => -(e.duration().as_millis() as f64),
    }
}

// ---------------------------------------------------------------------------
// Fuso horário local
// ---------------------------------------------------------------------------

/// Tabela de transições de um arquivo TZif (ou um deslocamento fixo).
/// Os deslocamentos são em segundos.
struct Tabela {
    tempos: Vec<i64>,
    idx: Vec<usize>,
    tipos: Vec<i64>,
}

impl Tabela {
    fn deslocamento_em(&self, segundos: i64) -> i64 {
        // Antes da primeira transição vale o tipo 0 (RFC 8536).
        let k = self.tempos.partition_point(|&x| x <= segundos);
        if k == 0 {
            self.tipos[0]
        } else {
            self.tipos[self.idx[k - 1]]
        }
    }
}

static FUSO: OnceLock<Tabela> = OnceLock::new();

/// Diferença do fuso local em ms (local - UTC) no instante `t` UTC.
pub fn fuso_local(t: f64) -> f64 {
    if !t.is_finite() {
        return 0.0;
    }
    let tab = FUSO.get_or_init(carregar_fuso);
    tab.deslocamento_em((t / 1000.0).floor() as i64) as f64 * 1000.0
}

/// Converte um instante UTC para o "relógio de parede" local.
pub fn local_de_utc(t: f64) -> f64 {
    t + fuso_local(t)
}

/// Converte um horário local para UTC.
pub fn utc_de_local(t: f64) -> f64 {
    t - fuso_local(t - fuso_local(t))
}

fn carregar_fuso() -> Tabela {
    let fixo = |s: i64| Tabela {
        tempos: Vec::new(),
        idx: Vec::new(),
        tipos: vec![s],
    };
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if !tz.is_empty() {
            if let Some(s) = fuso_textual(tz) {
                return fixo(s);
            }
            if !tz.contains("..") {
                if let Some(t) = ler_arquivo(&format!("/usr/share/zoneinfo/{}", tz)) {
                    return t;
                }
            }
        }
    }
    ler_arquivo("/etc/localtime").unwrap_or_else(|| fixo(0))
}

/// Lê um TZ textual sem regras de horário de verão. Retorna segundos.
/// Convenção: "UTC-3" (sinal depois do nome) vale -3 h literal; sem sinal
/// (formato POSIX, "EST5" ou "<-03>3") o número é o que fica a oeste, então
/// o deslocamento é negativo.
/// shortcut: regras de horário de verão do TZ não são lidas, "EST5EDT" cai
/// no arquivo, quando houver suporte a DST em texto subir para cá.
fn fuso_textual(tz: &str) -> Option<i64> {
    let b = tz.as_bytes();
    let (nome_ok, mut p) = if b.first() == Some(&b'<') {
        (true, tz.find('>')? + 1)
    } else {
        let n = b.iter().take_while(|c| c.is_ascii_alphabetic()).count();
        (n >= 3, n)
    };
    if !nome_ok {
        return None;
    }
    if p == b.len() {
        return if tz == "UTC" || tz == "GMT" { Some(0) } else { None };
    }
    let sinal: i64 = match b[p] {
        b'+' => {
            p += 1;
            1
        }
        b'-' => {
            p += 1;
            -1
        }
        _ => -1,
    };
    let mut partes = [0i64; 3];
    let mut k = 0;
    loop {
        let ini = p;
        while p < b.len() && b[p].is_ascii_digit() {
            p += 1;
        }
        if p == ini || p - ini > 2 || k >= 3 {
            return None;
        }
        partes[k] = tz[ini..p].parse::<i64>().ok()?;
        k += 1;
        if p < b.len() && b[p] == b':' {
            p += 1;
        } else {
            break;
        }
    }
    if p != b.len() || partes[0] > 24 || partes[1] > 59 || partes[2] > 59 {
        return None;
    }
    Some(sinal * (partes[0] * 3600 + partes[1] * 60 + partes[2]))
}

fn ler_arquivo(caminho: &str) -> Option<Tabela> {
    std::fs::read(caminho).ok().and_then(|b| ler_tzif(&b))
}

fn be_u32(b: &[u8], q: usize) -> u32 {
    u32::from_be_bytes([b[q], b[q + 1], b[q + 2], b[q + 3]])
}

fn be_i32(b: &[u8], q: usize) -> i64 {
    i32::from_be_bytes([b[q], b[q + 1], b[q + 2], b[q + 3]]) as i64
}

fn be_i64(b: &[u8], q: usize) -> i64 {
    i64::from_be_bytes([
        b[q],
        b[q + 1],
        b[q + 2],
        b[q + 3],
        b[q + 4],
        b[q + 5],
        b[q + 6],
        b[q + 7],
    ])
}

/// Cabeçalho TZif em `o`: [isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt].
fn cabecalho(b: &[u8], o: usize) -> Option<[usize; 6]> {
    if b.len() < o + 44 || b[o..o + 4] != b"TZif"[..] {
        return None;
    }
    let mut v = [0usize; 6];
    for (k, slot) in v.iter_mut().enumerate() {
        *slot = be_u32(b, o + 20 + 4 * k) as usize;
    }
    Some(v)
}

/// Lê um arquivo TZif (v1 ou v2+). Usa o bloco de 64 bits quando existe.
fn ler_tzif(b: &[u8]) -> Option<Tabela> {
    let v1 = cabecalho(b, 0)?;
    let (o, h, tam_t) = if b[4] == 0 {
        (0usize, v1, 4usize)
    } else {
        let tam = v1[3] * 5 + v1[4] * 6 + v1[5] + v1[2] * 8 + v1[1] + v1[0];
        (44 + tam, cabecalho(b, 44 + tam)?, 8usize)
    };
    let (tc, tp, cc) = (h[3], h[4], h[5]);
    let _ = cc;
    let mut p = o + 44;
    if b.len() < p + tc * (tam_t + 1) + tp * 6 {
        return None;
    }
    let mut tempos = Vec::with_capacity(tc);
    for k in 0..tc {
        let q = p + k * tam_t;
        tempos.push(if tam_t == 4 { be_i32(b, q) } else { be_i64(b, q) });
    }
    p += tc * tam_t;
    let idx: Vec<usize> = b[p..p + tc].iter().map(|&x| x as usize).collect();
    p += tc;
    let mut tipos = Vec::with_capacity(tp);
    for k in 0..tp {
        tipos.push(be_i32(b, p + k * 6));
    }
    if tipos.is_empty() || idx.iter().any(|&i| i >= tp) {
        return None;
    }
    Some(Tabela { tempos, idx, tipos })
}

// ---------------------------------------------------------------------------
// Calendário (algoritmos da especificação, seção 21.4.1)
// ---------------------------------------------------------------------------

fn bissexto(y: f64) -> bool {
    y % 4.0 == 0.0 && (y % 100.0 != 0.0 || y % 400.0 == 0.0)
}

/// DayFromYear: dia (0 = 1970-01-01) em que começa o ano `y`.
fn dia_do_ano_inicio(y: f64) -> f64 {
    365.0 * (y - 1970.0) + ((y - 1969.0) / 4.0).floor() - ((y - 1901.0) / 100.0).floor()
        + ((y - 1601.0) / 400.0).floor()
}

fn dias_no_mes(m: usize, bis: bool) -> f64 {
    if m == 1 && bis {
        29.0
    } else {
        DIAS_MES[m]
    }
}

fn dias_antes_do_mes(m: usize, bis: bool) -> f64 {
    DIAS_ACUM[m] + if bis && m >= 2 { 1.0 } else { 0.0 }
}

/// Dia inteiro (may be negative) de `t`.
fn dia(t: f64) -> f64 {
    (t / MS_POR_DIA).floor()
}

fn tempo_no_dia(t: f64) -> f64 {
    t.rem_euclid(MS_POR_DIA)
}

/// (ano, mês 0-11, dia 1-31) de um instante já finito.
fn decompor(t: f64) -> (f64, usize, f64) {
    let y = ano(t);
    let mut resto = dia(t) - dia_do_ano_inicio(y);
    let bis = bissexto(y);
    for m in 0..12 {
        let n = dias_no_mes(m, bis);
        if resto < n {
            return (y, m, resto + 1.0);
        }
        resto -= n;
    }
    (y, 11, resto + 1.0)
}

/// Ano de `t` (YearFromTime).
pub fn ano(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    let d = dia(t);
    let mut y = (d / 365.2425).floor() + 1970.0;
    while dia_do_ano_inicio(y) > d {
        y -= 1.0;
    }
    while dia_do_ano_inicio(y + 1.0) <= d {
        y += 1.0;
    }
    y
}

/// Mês de `t`, 0 a 11.
pub fn mes(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    decompor(t).1 as f64
}

/// Dia do mês de `t`, 1 a 31.
pub fn data(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    decompor(t).2
}

/// Dia da semana de `t`, 0 = domingo.
pub fn dia_semana(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    (dia(t) + 4.0).rem_euclid(7.0)
}

/// Número do dia desde 1970 (Day).
pub fn dia_de(t: f64) -> f64 {
    dia(t)
}

/// Milissegundos dentro do dia (0 a 86 399 999).
pub fn tempo_do_dia(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    tempo_no_dia(t)
}

/// Hora do dia, 0 a 23.
pub fn horas(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    (tempo_no_dia(t) / MS_POR_HORA).floor()
}

/// Minuto da hora, 0 a 59.
pub fn minutos(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    (tempo_no_dia(t) / MS_POR_MINUTO).floor().rem_euclid(60.0)
}

/// Segundo do minuto, 0 a 59.
pub fn segundos(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    (tempo_no_dia(t) / 1000.0).floor().rem_euclid(60.0)
}

/// Milissegundo do segundo, 0 a 999.
pub fn ms(t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    tempo_no_dia(t).rem_euclid(1000.0)
}

/// MakeTime: horas, minutos, segundos e milissegundos viram ms dentro do dia.
pub fn fazer_tempo(h: f64, m: f64, s: f64, milis: f64) -> f64 {
    if !(h.is_finite() && m.is_finite() && s.is_finite() && milis.is_finite()) {
        return f64::NAN;
    }
    h.trunc() * MS_POR_HORA + m.trunc() * MS_POR_MINUTO + s.trunc() * 1000.0 + milis.trunc()
}

/// MakeDay: ano, mês (0-11, aceita valores fora da faixa) e dia viram número do dia.
pub fn fazer_dia(y: f64, m: f64, d: f64) -> f64 {
    if !(y.is_finite() && m.is_finite() && d.is_finite()) {
        return f64::NAN;
    }
    let (y, m, d) = (y.trunc(), m.trunc(), d.trunc());
    let ym = y + (m / 12.0).floor();
    let mn = m.rem_euclid(12.0) as usize;
    dia_do_ano_inicio(ym) + dias_antes_do_mes(mn, bissexto(ym)) + d - 1.0
}

/// MakeDate: número do dia vezes o dia em ms, mais o tempo.
pub fn fazer_data(dia_: f64, tempo: f64) -> f64 {
    if !(dia_.is_finite() && tempo.is_finite()) {
        return f64::NAN;
    }
    dia_ * MS_POR_DIA + tempo
}

/// TimeClip: fora de +-8.64e15 vira NaN; senão trunca e zera o sinal.
pub fn cortar_tempo(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > 8.64e15 {
        f64::NAN
    } else {
        t.trunc() + 0.0
    }
}

// ---------------------------------------------------------------------------
// Campos de um instante já deslocado para o relógio desejado
// ---------------------------------------------------------------------------

struct Campos {
    ano: i64,
    mes: usize,
    dia: i64,
    hora: i64,
    minuto: i64,
    segundo: i64,
    milis: i64,
    semana: usize,
}

fn campos(t: f64) -> Campos {
    let (y, m, d) = decompor(t);
    let tempo = tempo_no_dia(t).floor() as i64;
    Campos {
        ano: y as i64,
        mes: m,
        dia: d as i64,
        hora: tempo / 3_600_000,
        minuto: tempo / 60_000 % 60,
        segundo: tempo / 1_000 % 60,
        milis: tempo % 1_000,
        semana: dia_semana(t) as usize,
    }
}

fn ano_texto(y: i64) -> String {
    if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{:04}", y)
    }
}

/// "GMT-0300" ou, com dois pontos, "GMT-03:00". `off_ms` é o deslocamento em ms.
fn texto_gmt(off_ms: f64, dois_pontos: bool) -> String {
    let min = (off_ms / MS_POR_MINUTO).trunc() as i64;
    let sinal = if min < 0 { '-' } else { '+' };
    let a = min.abs();
    if dois_pontos {
        format!("GMT{}{:02}:{:02}", sinal, a / 60, a % 60)
    } else {
        format!("GMT{}{:02}{:02}", sinal, a / 60, a % 60)
    }
}

// ---------------------------------------------------------------------------
// Textos (toString, toISOString, toLocaleString...)
// ---------------------------------------------------------------------------

/// Date.prototype.toString: "Tue Mar 05 2024 14:03:00 GMT-0300 (GMT-03:00)".
pub fn para_texto(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    format!("{} {}", para_texto_data(t), para_texto_hora(t))
}

/// toDateString: "Tue Mar 05 2024".
pub fn para_texto_data(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    let c = campos(local_de_utc(t));
    format!(
        "{} {} {:02} {}",
        NOMES_DIA[c.semana],
        NOMES_MES[c.mes],
        c.dia,
        ano_texto(c.ano)
    )
}

/// toTimeString: "14:03:00 GMT-0300 (GMT-03:00)".
pub fn para_texto_hora(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    let off = fuso_local(t);
    let c = campos(t + off);
    format!(
        "{:02}:{:02}:{:02} {} ({})",
        c.hora,
        c.minuto,
        c.segundo,
        texto_gmt(off, false),
        texto_gmt(off, true)
    )
}

/// toUTCString: "Tue, 05 Mar 2024 17:03:00 GMT".
pub fn para_texto_utc(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    let c = campos(t);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        NOMES_DIA[c.semana],
        c.dia,
        NOMES_MES[c.mes],
        ano_texto(c.ano),
        c.hora,
        c.minuto,
        c.segundo
    )
}

/// toISOString: "2024-03-05T17:03:00.000Z". Anos fora de 0..=9999 usam sinal e 6 dígitos.
pub fn para_iso(t: f64) -> Option<String> {
    if !t.is_finite() {
        return None;
    }
    let c = campos(t);
    let texto_ano = if (0..=9999).contains(&c.ano) {
        format!("{:04}", c.ano)
    } else if c.ano < 0 {
        format!("-{:06}", -c.ano)
    } else {
        format!("+{:06}", c.ano)
    };
    Some(format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        texto_ano,
        c.mes + 1,
        c.dia,
        c.hora,
        c.minuto,
        c.segundo,
        c.milis
    ))
}

/// toLocaleString em estilo pt-BR: "05/03/2024, 14:03:00" (hora local).
pub fn para_texto_local(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    format!(
        "{}, {}",
        para_texto_local_data(t),
        para_texto_local_hora(t)
    )
}

/// "05/03/2024" (hora local).
pub fn para_texto_local_data(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    let c = campos(local_de_utc(t));
    format!("{:02}/{:02}/{}", c.dia, c.mes + 1, ano_texto(c.ano))
}

/// "14:03:00" (hora local).
pub fn para_texto_local_hora(t: f64) -> String {
    if !t.is_finite() {
        return "Invalid Date".to_string();
    }
    let c = campos(local_de_utc(t));
    format!("{:02}:{:02}:{:02}", c.hora, c.minuto, c.segundo)
}

// ---------------------------------------------------------------------------
// Date.parse
// ---------------------------------------------------------------------------

/// Date.parse. Aceita o formato ISO e os formatos que os navegadores aceitam.
/// Inválido = NaN.
pub fn analisar(s: &str) -> f64 {
    let s = s.trim();
    analisar_iso(s)
        .or_else(|| analisar_legado(s))
        .unwrap_or(f64::NAN)
}

/// Lê exatamente `n` dígitos em `p` e avança.
fn digitos_fixos(b: &[u8], p: &mut usize, n: usize) -> Option<u32> {
    let mut v = 0u32;
    for _ in 0..n {
        let c = *b.get(*p)?;
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
        *p += 1;
    }
    Some(v)
}

/// Lê um inteiro de um ou mais dígitos (até 9). Devolve (valor, quantidade de dígitos).
fn ler_inteiro(b: &[u8], p: &mut usize) -> Option<(i64, usize)> {
    let ini = *p;
    let mut v: i64 = 0;
    while *p < b.len() && b[*p].is_ascii_digit() {
        v = v * 10 + (b[*p] - b'0') as i64;
        *p += 1;
        if *p - ini > 9 {
            return None;
        }
    }
    if *p == ini {
        None
    } else {
        Some((v, *p - ini))
    }
}

/// Formato ISO da especificação. Só data = UTC; data+hora sem fuso = local.
fn analisar_iso(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut p = 0usize;
    let anoi: i64 = if b.first() == Some(&b'+') || b.first() == Some(&b'-') {
        let neg = b[0] == b'-';
        p = 1;
        let v = digitos_fixos(b, &mut p, 6)? as i64;
        if neg && v == 0 {
            return None;
        }
        if neg {
            -v
        } else {
            v
        }
    } else {
        digitos_fixos(b, &mut p, 4)? as i64
    };

    let mut mesi: u32 = 1;
    let mut diai: u32 = 1;
    if b.get(p) == Some(&b'-') {
        p += 1;
        mesi = digitos_fixos(b, &mut p, 2)?;
        if b.get(p) == Some(&b'-') {
            p += 1;
            diai = digitos_fixos(b, &mut p, 2)?;
        }
    }

    let tem_hora = b.get(p) == Some(&b'T');
    let mut tempo = (0u32, 0u32, 0u32, 0u32);
    if tem_hora {
        p += 1;
        let hh = digitos_fixos(b, &mut p, 2)?;
        if b.get(p) != Some(&b':') {
            return None;
        }
        p += 1;
        let mi = digitos_fixos(b, &mut p, 2)?;
        let mut ss = 0u32;
        let mut mil = 0u32;
        if b.get(p) == Some(&b':') {
            p += 1;
            ss = digitos_fixos(b, &mut p, 2)?;
            if b.get(p) == Some(&b'.') {
                p += 1;
                let ini = p;
                let mut v = 0u32;
                while p < b.len() && b[p].is_ascii_digit() {
                    if p - ini < 3 {
                        v = v * 10 + (b[p] - b'0') as u32;
                    }
                    p += 1;
                }
                let n = (p - ini).min(3);
                if n == 0 {
                    return None;
                }
                mil = v * 10u32.pow((3 - n) as u32);
            }
        }
        tempo = (hh, mi, ss, mil);
    }

    // Fuso: "Z" ou ±HH:MM.
    let mut offset: Option<i64> = None; // ms
    match b.get(p) {
        None => {}
        Some(&b'Z') => {
            offset = Some(0);
            p += 1;
        }
        Some(&sinal) if sinal == b'+' || sinal == b'-' => {
            p += 1;
            let hh = digitos_fixos(b, &mut p, 2)? as i64;
            if b.get(p) != Some(&b':') {
                return None;
            }
            p += 1;
            let mm = digitos_fixos(b, &mut p, 2)? as i64;
            if hh > 23 || mm > 59 {
                return None;
            }
            let total = (hh * 60 + mm) * 60_000;
            offset = Some(if sinal == b'-' { -total } else { total });
        }
        _ => return None,
    }
    if p != b.len() || (offset.is_some() && !tem_hora) {
        return None;
    }

    let (hh, mi, ss, mil) = tempo;
    if !(1..=12).contains(&mesi)
        || diai < 1
        || (diai as f64) > dias_no_mes((mesi - 1) as usize, bissexto(anoi as f64))
    {
        return None;
    }
    if hh > 24 || mi > 59 || ss > 59 || (hh == 24 && (mi | ss | mil) != 0) {
        return None;
    }

    let dia_n = fazer_dia(anoi as f64, (mesi - 1) as f64, diai as f64);
    let t = fazer_data(
        dia_n,
        fazer_tempo(hh as f64, mi as f64, ss as f64, mil as f64),
    );
    let t = match offset {
        Some(o) => t - o as f64,
        None if tem_hora => utc_de_local(t),
        None => t,
    };
    Some(cortar_tempo(t))
}

fn nome_mes(w: &str) -> Option<usize> {
    if w.len() < 3 {
        return None;
    }
    MESES_MIN.iter().position(|m| w.starts_with(m))
}

fn nome_dia_semana(w: &str) -> bool {
    w.len() >= 3 && DIAS_MIN.iter().any(|d| w.starts_with(d))
}

/// Formatos livres dos navegadores: "Tue Mar 05 2024 14:03:00 GMT-0300 (...)",
/// "Tue, 05 Mar 2024 14:03:00 GMT", "Mar 5, 2024", "2024/03/05 10:00".
/// Sem fuso explícito vale a hora local.
fn analisar_legado(s: &str) -> Option<f64> {
    // Texto entre parênteses é comentário.
    let mut limpo = String::new();
    let mut prof = 0;
    for c in s.chars() {
        match c {
            '(' => prof += 1,
            ')' if prof > 0 => prof -= 1,
            _ if prof == 0 => limpo.push(c),
            _ => {}
        }
    }
    let b = limpo.as_bytes();

    let mut p = 0usize;
    let mut nums: Vec<(i64, usize)> = Vec::new();
    let mut mes_nome: Option<usize> = None;
    let mut tempo: Option<(i64, i64, i64, i64)> = None;
    let mut ampm: Option<bool> = None; // Some(true) = PM
    let mut gmt = false;
    let mut offset: Option<i64> = None; // ms

    while p < b.len() {
        let c = b[p];
        if c.is_ascii_digit() {
            let (v, nd) = ler_inteiro(b, &mut p)?;
            if p < b.len() && b[p] == b':' {
                if tempo.is_some() {
                    return None;
                }
                p += 1;
                let (mi, _) = ler_inteiro(b, &mut p)?;
                let mut ss: i64 = 0;
                let mut mil: i64 = 0;
                if p < b.len() && b[p] == b':' {
                    p += 1;
                    ss = ler_inteiro(b, &mut p)?.0;
                    if p < b.len() && b[p] == b'.' {
                        p += 1;
                        let (fr, nf) = ler_inteiro(b, &mut p)?;
                        mil = if nf >= 3 {
                            fr / 10_i64.pow((nf - 3) as u32)
                        } else {
                            fr * 10_i64.pow((3 - nf) as u32)
                        };
                    }
                }
                tempo = Some((v, mi, ss, mil));
            } else {
                nums.push((v, nd));
            }
        } else if c.is_ascii_alphabetic() {
            let ini = p;
            while p < b.len() && b[p].is_ascii_alphabetic() {
                p += 1;
            }
            let w = String::from_utf8_lossy(&b[ini..p]).to_ascii_lowercase();
            if w == "am" {
                ampm = Some(false);
            } else if w == "pm" {
                ampm = Some(true);
            } else if w == "gmt" || w == "utc" || w == "ut" || w == "z" {
                gmt = true;
                offset = offset.or(Some(0));
            } else if let Some(m) = nome_mes(&w) {
                if mes_nome.is_some() {
                    return None;
                }
                mes_nome = Some(m);
            } else if !nome_dia_semana(&w) {
                return None;
            }
        } else if (c == b'+' || c == b'-')
            && p + 1 < b.len()
            && b[p + 1].is_ascii_digit()
            && (tempo.is_some() || gmt)
        {
            // Deslocamento depois da hora ou do GMT.
            let sinal: i64 = if c == b'-' { -1 } else { 1 };
            p += 1;
            let (v, nd) = ler_inteiro(b, &mut p)?;
            let (hh, mm) = if p < b.len() && b[p] == b':' {
                p += 1;
                let (mm, _) = ler_inteiro(b, &mut p)?;
                (v, mm)
            } else if nd == 4 {
                (v / 100, v % 100)
            } else if nd <= 2 {
                (v, 0)
            } else {
                return None;
            };
            if hh > 24 || mm > 59 {
                return None;
            }
            offset = Some(sinal * (hh * 60 + mm) * 60_000);
        } else if matches!(c, b' ' | b',' | b'/' | b'-' | b'.' | b'\t') {
            p += 1;
        } else {
            return None;
        }
    }

    let (anoi, mes, dia_m) = match mes_nome {
        Some(m) => {
            if nums.len() != 2 {
                return None;
            }
            let (a, c) = (nums[0], nums[1]);
            if a.1 >= 3 || a.0 > 31 {
                (a.0, m as i64, c.0)
            } else if c.1 >= 3 || c.0 > 31 {
                (c.0, m as i64, a.0)
            } else {
                return None;
            }
        }
        None => {
            if nums.len() != 3 {
                return None;
            }
            if nums[0].1 >= 3 {
                // AAAA/MM/DD
                (nums[0].0, nums[1].0 - 1, nums[2].0)
            } else {
                // MM/DD/AAAA
                (nums[2].0, nums[0].0 - 1, nums[1].0)
            }
        }
    };
    if !(0..12).contains(&mes) || dia_m < 1 {
        return None;
    }
    if (dia_m as f64) > dias_no_mes(mes as usize, bissexto(anoi as f64)) {
        return None;
    }

    let (mut hh, mi, ss, mil) = tempo.unwrap_or((0, 0, 0, 0));
    match ampm {
        Some(true) if hh < 12 => hh += 12,
        Some(false) if hh == 12 => hh = 0,
        _ => {}
    }
    if hh > 24 || mi > 59 || ss > 59 {
        return None;
    }

    let dia_n = fazer_dia(anoi as f64, mes as f64, dia_m as f64);
    let t = fazer_data(
        dia_n,
        fazer_tempo(hh as f64, mi as f64, ss as f64, mil as f64),
    );
    // Sem fuso explícito, o texto é hora local.
    let t = match offset {
        Some(o) => t - o as f64,
        None => utc_de_local(t),
    };
    Some(cortar_tempo(t))
}

// ---------------------------------------------------------------------------
// Testes (independentes do fuso da máquina: só usam funções UTC)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn dia_zero_e_quinta_feira() {
        assert_eq!(fazer_dia(1970.0, 0.0, 1.0), 0.0);
        assert_eq!(dia_semana(0.0), 4.0);
        assert_eq!(ano(0.0), 1970.0);
        assert_eq!(mes(0.0), 0.0);
        assert_eq!(data(0.0), 1.0);
    }

    #[test]
    fn dia_anterior_ao_epoca() {
        let t = -MS_POR_DIA;
        assert_eq!(ano(t), 1969.0);
        assert_eq!(mes(t), 11.0);
        assert_eq!(data(t), 31.0);
        assert_eq!(dia_semana(t), 3.0);
    }

    #[test]
    fn ano_bissexto_2000() {
        assert_eq!(fazer_dia(2000.0, 1.0, 29.0), 11016.0);
        let t = analisar("2000-02-29T00:00:00Z");
        assert_eq!(t, 11016.0 * MS_POR_DIA);
        assert_eq!(dia_semana(t), 2.0);
        assert_eq!(ano(t), 2000.0);
        assert_eq!(mes(t), 1.0);
        assert_eq!(data(t), 29.0);
    }

    #[test]
    fn ano_1900_nao_e_bissexto() {
        assert_eq!(fazer_dia(1900.0, 0.0, 1.0), -25567.0);
        assert_eq!(fazer_dia(1900.0, 2.0, 1.0), -25508.0);
    }

    #[test]
    fn ano_negativo() {
        let t = fazer_data(fazer_dia(-1.0, 0.0, 1.0), 0.0);
        assert_eq!(ano(t), -1.0);
        assert_eq!(mes(t), 0.0);
        assert_eq!(data(t), 1.0);
        assert_eq!(para_iso(t).as_deref(), Some("-000001-01-01T00:00:00.000Z"));
    }

    #[test]
    fn mes_fora_da_faixa_rola_o_ano() {
        assert_eq!(fazer_dia(2024.0, 12.0, 1.0), fazer_dia(2025.0, 0.0, 1.0));
        assert_eq!(fazer_dia(2024.0, -1.0, 1.0), fazer_dia(2023.0, 11.0, 1.0));
    }

    #[test]
    fn dia_da_semana_de_2024() {
        // 2024-03-05 foi terça-feira.
        let t = analisar("2024-03-05T17:03:00Z");
        assert_eq!(dia_semana(t), 2.0);
        assert_eq!(ano(t), 2024.0);
        assert_eq!(mes(t), 2.0);
        assert_eq!(data(t), 5.0);
    }

    #[test]
    fn horas_minutos_segundos_e_ms() {
        let t = analisar("2024-03-05T17:03:09.250Z");
        assert_eq!(horas(t), 17.0);
        assert_eq!(minutos(t), 3.0);
        assert_eq!(segundos(t), 9.0);
        assert_eq!(ms(t), 250.0);
    }

    #[test]
    fn fazer_tempo_soma_os_campos() {
        assert_eq!(fazer_tempo(1.0, 2.0, 3.0, 4.0), 3_723_004.0);
        assert!(fazer_tempo(f64::NAN, 0.0, 0.0, 0.0).is_nan());
    }

    #[test]
    fn analisar_iso_com_z() {
        assert_eq!(
            para_iso(analisar("2024-03-05T17:03:00Z")).as_deref(),
            Some("2024-03-05T17:03:00.000Z")
        );
    }

    #[test]
    fn analisar_iso_com_deslocamento() {
        assert_eq!(
            para_iso(analisar("2024-03-05T14:03:00-03:00")).as_deref(),
            Some("2024-03-05T17:03:00.000Z")
        );
        assert_eq!(
            para_iso(analisar("2024-03-05T23:33:00+05:30")).as_deref(),
            Some("2024-03-05T18:03:00.000Z")
        );
    }

    #[test]
    fn analisar_so_data_e_utc() {
        assert_eq!(
            para_iso(analisar("2024-03-05")).as_deref(),
            Some("2024-03-05T00:00:00.000Z")
        );
        assert_eq!(
            para_iso(analisar("2024-03-05T10:00:00.5Z")).as_deref(),
            Some("2024-03-05T10:00:00.500Z")
        );
    }

    #[test]
    fn analisar_invalidos() {
        assert!(analisar("2024-02-30").is_nan());
        assert!(analisar("2024-13-01").is_nan());
        assert!(analisar("abc").is_nan());
        assert!(analisar("").is_nan());
    }

    #[test]
    fn analisar_formatos_do_navegador() {
        let ref_utc = analisar("2024-03-05T17:03:00Z");
        assert_eq!(analisar("Tue, 05 Mar 2024 17:03:00 GMT"), ref_utc);
        assert_eq!(
            analisar("Tue Mar 05 2024 14:03:00 GMT-0300 (horário)"),
            ref_utc
        );
        assert_eq!(
            analisar("March 5, 2024 14:03:00 GMT-03:00"),
            ref_utc
        );
        assert_eq!(
            analisar("March 5, 2024 10:00:00 GMT+0200"),
            analisar("2024-03-05T08:00:00Z")
        );
    }

    #[test]
    fn para_texto_utc_formata() {
        assert_eq!(
            para_texto_utc(analisar("2024-03-05T17:03:00Z")),
            "Tue, 05 Mar 2024 17:03:00 GMT"
        );
        assert_eq!(para_texto_utc(f64::NAN), "Invalid Date");
    }

    #[test]
    fn para_iso_de_nan_e_none() {
        assert_eq!(para_iso(f64::NAN), None);
    }

    #[test]
    fn cortar_tempo_limites() {
        assert_eq!(cortar_tempo(8.64e15), 8.64e15);
        assert!(cortar_tempo(8.64e15 + 1.0).is_nan());
        assert_eq!(cortar_tempo(1.9), 1.0);
        assert_eq!(cortar_tempo(-1.9), -1.0);
        assert!(cortar_tempo(f64::INFINITY).is_nan());
    }
}
