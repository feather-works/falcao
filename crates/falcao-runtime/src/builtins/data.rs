//! Date: o objeto de data sobre o módulo `crate::datas` (calendário, fuso e textos).
//!
//! O tempo fica no interno 0 como número (ms desde 1970 UTC; NaN = data inválida).

use crate::conversoes::Dica;
use crate::datas;
use crate::formas::{attr, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;
    let op = m.intr.objeto_proto;
    let f = m.forma_para(Classe::Comum, op, 0);
    let dp = m.novo_objeto(f);
    let ctor = m.nativa("Date", 7, date_construtor, true);
    m.definir_propria(ctor, fixos::PROTOTYPE, dp, 0);
    m.definir_propria(dp, fixos::CONSTRUCTOR, ctor, attr::METODO);
    let k = m.chave("Date");
    m.definir_propria(g, k, ctor, attr::METODO);
    m.metodo(ctor, "now", 0, date_now);
    m.metodo(ctor, "parse", 1, date_parse);
    m.metodo(ctor, "UTC", 7, date_utc);

    let leituras: &[(&'static str, FnNativa)] = &[
        ("getTime", get_tempo),
        ("valueOf", get_tempo),
        ("getFullYear", get_ano),
        ("getMonth", get_mes),
        ("getDate", get_dia),
        ("getDay", get_dia_semana),
        ("getHours", get_hora),
        ("getMinutes", get_minuto),
        ("getSeconds", get_segundo),
        ("getMilliseconds", get_ms),
        ("getYear", get_ano_legado),
        ("getUTCFullYear", get_ano_utc),
        ("getUTCMonth", get_mes_utc),
        ("getUTCDate", get_dia_utc),
        ("getUTCDay", get_dia_semana_utc),
        ("getUTCHours", get_hora_utc),
        ("getUTCMinutes", get_minuto_utc),
        ("getUTCSeconds", get_segundo_utc),
        ("getUTCMilliseconds", get_ms_utc),
        ("getTimezoneOffset", get_deslocamento),
        ("setTime", set_tempo),
        ("setMilliseconds", set_ms),
        ("setSeconds", set_segundos),
        ("setMinutes", set_minutos),
        ("setHours", set_horas),
        ("setDate", set_dia),
        ("setMonth", set_mes),
        ("setFullYear", set_ano),
        ("setYear", set_ano_legado),
        ("setUTCMilliseconds", set_ms_utc),
        ("setUTCSeconds", set_segundos_utc),
        ("setUTCMinutes", set_minutos_utc),
        ("setUTCHours", set_horas_utc),
        ("setUTCDate", set_dia_utc),
        ("setUTCMonth", set_mes_utc),
        ("setUTCFullYear", set_ano_utc),
        ("toISOString", to_iso),
        ("toJSON", to_json),
        ("toString", d_para_texto),
        ("toDateString", d_para_texto_data),
        ("toTimeString", d_para_texto_hora),
        ("toUTCString", d_para_texto_utc),
        ("toGMTString", d_para_texto_utc),
        ("toLocaleString", d_local),
        ("toLocaleDateString", d_local_data),
        ("toLocaleTimeString", d_local_hora),
    ];
    for &(nome, f) in leituras {
        m.metodo(dp, nome, 0, f);
    }
    let tp = m.nativa("[Symbol.toPrimitive]", 1, d_to_primitive, false);
    m.definir_propria(dp, simb::TO_PRIMITIVE, tp, attr::CONFIGURAVEL);
}

// ---------------------------------------------------------------- auxiliares

/// Tempo de um Date (ou TypeError se `this` não é Date).
fn tempo_de(m: &mut Motor, this: Valor) -> Res<f64> {
    if m.e_objeto(this) && m.formas.get(m.forma(this)).classe == Classe::Data {
        let v = Valor(m.interno(this, 0));
        return Ok(m.f64_de(v));
    }
    m.erro_tipo("this is not a Date object.")
}

fn gravar_tempo(m: &mut Motor, o: Valor, t: f64) {
    let v = m.numero(t);
    m.por_interno_valor(o, 0, v);
}

/// Argumentos numéricos (até `quantos`); sem argumento vale NaN, como no ECMAScript.
fn args_nums(m: &mut Motor, a: Args, quantos: u32) -> Res<Vec<f64>> {
    let n = quantos.min(a.n.max(1));
    let mut v = Vec::with_capacity(n as usize);
    for i in 0..n {
        let x = m.arg(a, i);
        v.push(m.para_numero(x)?);
    }
    Ok(v)
}

/// Ano, mês, dia, h, min, s, ms (padrões: mês 0, dia 1, resto 0). Ano 0..=99 vira 1900+.
fn montar(vals: &[f64]) -> f64 {
    let c = |i: usize, padrao: f64| vals.get(i).copied().unwrap_or(padrao);
    let mut ano = c(0, f64::NAN);
    if ano.is_finite() {
        let i = ano.trunc();
        if (0.0..=99.0).contains(&i) {
            ano = 1900.0 + i;
        }
    }
    let dia = datas::fazer_dia(ano, c(1, 0.0), c(2, 1.0));
    let tempo = datas::fazer_tempo(c(3, 0.0), c(4, 0.0), c(5, 0.0), c(6, 0.0));
    datas::fazer_data(dia, tempo)
}

fn criar_data(m: &mut Motor, nt: Valor, t: f64) -> Res<Valor> {
    let padrao = m.chamado;
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = if m.e_objeto(p) { p } else { m.obter(padrao, ChaveProp::Nome(fixos::PROTOTYPE))? };
    let f = m.forma_para(Classe::Data, p, 0);
    let o = m.novo_objeto(f);
    gravar_tempo(m, o, t);
    Ok(o)
}

// ---------------------------------------------------------------- construtor e estáticos

fn date_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        let s = datas::para_texto(datas::agora());
        return Ok(m.texto(&s));
    }
    let t = if a.n == 0 {
        datas::agora()
    } else if a.n == 1 {
        let v = m.arg(a, 0);
        if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Data {
            tempo_de(m, v)?
        } else {
            let p = m.para_primitivo(v, Dica::Padrao)?;
            if m.e_texto(p) {
                let s = m.texto_rust(p);
                datas::analisar(&s)
            } else {
                let x = m.para_numero(p)?;
                datas::cortar_tempo(x)
            }
        }
    } else {
        let vals = args_nums(m, a, 7)?;
        datas::cortar_tempo(datas::utc_de_local(montar(&vals)))
    };
    criar_data(m, nt, t)
}

fn date_now(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(m.numero(datas::agora()))
}

fn date_parse(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    let s = m.para_texto_rust(v)?;
    Ok(m.numero(datas::analisar(&s)))
}

fn date_utc(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let vals = args_nums(m, a, 7)?;
    Ok(m.numero(datas::cortar_tempo(montar(&vals))))
}

// ---------------------------------------------------------------- leituras

fn get_tempo(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = tempo_de(m, this)?;
    Ok(m.numero(t))
}

/// Leitura de um campo: `$local` escolhe hora local (true) ou UTC (false).
macro_rules! leitor {
    ($nome:ident, $f:expr, $local:expr) => {
        fn $nome(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
            let t = tempo_de(m, this)?;
            let t = if $local { datas::local_de_utc(t) } else { t };
            let r = ($f)(t);
            Ok(m.numero(r))
        }
    };
}

leitor!(get_ano, datas::ano, true);
leitor!(get_ano_utc, datas::ano, false);
leitor!(get_ano_legado, |t: f64| datas::ano(t) - 1900.0, true);
leitor!(get_mes, datas::mes, true);
leitor!(get_mes_utc, datas::mes, false);
leitor!(get_dia, datas::data, true);
leitor!(get_dia_utc, datas::data, false);
leitor!(get_dia_semana, datas::dia_semana, true);
leitor!(get_dia_semana_utc, datas::dia_semana, false);
leitor!(get_hora, datas::horas, true);
leitor!(get_hora_utc, datas::horas, false);
leitor!(get_minuto, datas::minutos, true);
leitor!(get_minuto_utc, datas::minutos, false);
leitor!(get_segundo, datas::segundos, true);
leitor!(get_segundo_utc, datas::segundos, false);
leitor!(get_ms, datas::ms, true);
leitor!(get_ms_utc, datas::ms, false);

/// (UTC - local) em minutos: `-fuso`.
fn get_deslocamento(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = tempo_de(m, this)?;
    let r = (t - datas::local_de_utc(t)) / 60_000.0;
    Ok(m.numero(r))
}

// ---------------------------------------------------------------- ajustes

/// Troca os campos a partir de `inicio` (0 ano, 1 mês, 2 dia, 3 h, 4 min, 5 s, 6 ms) pelos
/// valores dados e grava. `ano` = setFullYear (data NaN vira 1970 em vez de NaN).
fn ajustar(m: &mut Motor, this: Valor, vals: &[f64], inicio: usize, local: bool, ano: bool) -> Res<Valor> {
    let t0 = tempo_de(m, this)?;
    let base = if !t0.is_nan() {
        if local {
            datas::local_de_utc(t0)
        } else {
            t0
        }
    } else if ano {
        0.0
    } else {
        return Ok(m.numero(f64::NAN));
    };
    let mut p = [
        datas::ano(base),
        datas::mes(base),
        datas::data(base),
        datas::horas(base),
        datas::minutos(base),
        datas::segundos(base),
        datas::ms(base),
    ];
    for (i, &v) in vals.iter().enumerate() {
        p[inicio + i] = v;
    }
    let novo = datas::fazer_data(datas::fazer_dia(p[0], p[1], p[2]), datas::fazer_tempo(p[3], p[4], p[5], p[6]));
    let r = datas::cortar_tempo(if local { datas::utc_de_local(novo) } else { novo });
    gravar_tempo(m, this, r);
    Ok(m.numero(r))
}

/// Setter com a quantidade de argumentos que ele aceita (`quantos`).
macro_rules! ajustador {
    ($nome:ident, $inicio:expr, $quantos:expr, $local:expr, $ano:expr) => {
        fn $nome(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
            tempo_de(m, this)?;
            let vals = args_nums(m, a, $quantos)?;
            ajustar(m, this, &vals, $inicio, $local, $ano)
        }
    };
}

ajustador!(set_ms, 6, 1, true, false);
ajustador!(set_ms_utc, 6, 1, false, false);
ajustador!(set_segundos, 5, 2, true, false);
ajustador!(set_segundos_utc, 5, 2, false, false);
ajustador!(set_minutos, 4, 3, true, false);
ajustador!(set_minutos_utc, 4, 3, false, false);
ajustador!(set_horas, 3, 4, true, false);
ajustador!(set_horas_utc, 3, 4, false, false);
ajustador!(set_dia, 2, 1, true, false);
ajustador!(set_dia_utc, 2, 1, false, false);
ajustador!(set_mes, 1, 2, true, false);
ajustador!(set_mes_utc, 1, 2, false, false);
ajustador!(set_ano, 0, 3, true, true);
ajustador!(set_ano_utc, 0, 3, false, true);

fn set_tempo(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    tempo_de(m, this)?;
    let vals = args_nums(m, a, 1)?;
    let r = datas::cortar_tempo(vals[0]);
    gravar_tempo(m, this, r);
    Ok(m.numero(r))
}

/// setYear: ano 0..=99 vira 1900+; NaN deixa a data inválida.
fn set_ano_legado(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    tempo_de(m, this)?;
    let vals = args_nums(m, a, 1)?;
    let y = vals[0];
    if y.is_nan() {
        gravar_tempo(m, this, f64::NAN);
        return Ok(m.numero(f64::NAN));
    }
    let i = y.trunc();
    let yy = if (0.0..=99.0).contains(&i) { 1900.0 + i } else { y };
    ajustar(m, this, &[yy], 0, true, true)
}

// ---------------------------------------------------------------- textos

fn to_iso(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = tempo_de(m, this)?;
    let s = match datas::para_iso(t) {
        Some(s) => s,
        None => return m.erro_alcance("Invalid time value"),
    };
    Ok(m.texto(&s))
}

/// toJSON: null para data inválida, senão o toISOString. shortcut: não chama toISOString do objeto.
fn to_json(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = tempo_de(m, this)?;
    match datas::para_iso(t) {
        Some(s) => Ok(m.texto(&s)),
        None => Ok(Valor::NULL),
    }
}

macro_rules! texto_de {
    ($nome:ident, $f:expr) => {
        fn $nome(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
            let t = tempo_de(m, this)?;
            let s = ($f)(t);
            Ok(m.texto(&s))
        }
    };
}

texto_de!(d_para_texto, datas::para_texto);
texto_de!(d_para_texto_data, datas::para_texto_data);
texto_de!(d_para_texto_hora, datas::para_texto_hora);
texto_de!(d_para_texto_utc, datas::para_texto_utc);
texto_de!(d_local, datas::para_texto_local);
texto_de!(d_local_data, datas::para_texto_local_data);
texto_de!(d_local_hora, datas::para_texto_local_hora);

/// `[Symbol.toPrimitive]`: "string"/"default" dá texto, "number" dá o tempo.
/// shortcut: só funciona em Date (não faz OrdinaryToPrimitive em outros objetos).
fn d_to_primitive(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = tempo_de(m, this)?;
    let h = m.arg(a, 0);
    let dica = if m.e_texto(h) { m.texto_rust(h) } else { String::new() };
    match dica.as_str() {
        "string" | "default" => {
            let s = datas::para_texto(t);
            Ok(m.texto(&s))
        }
        "number" => Ok(m.numero(t)),
        _ => {
            let d = m.inspecionar(h, 0);
            m.erro_tipo(&format!("Invalid hint: {d}"))
        }
    }
}
