//! ArrayBuffer, SharedArrayBuffer, os arrays tipados (`Uint8Array`…), DataView e Atomics.
//!
//! Os bytes moram na gaiola, num bloco `BYTES` ([forma][tamanho][bytes…]) guardado no campo
//! interno 0 do ArrayBuffer (undefined = desligado); redimensionar troca o bloco. Array tipado e
//! DataView guardam [buffer][tipo][deslocamento em bytes][comprimento] (em itens no array
//! tipado, em bytes no DataView; `AUTO` = acompanha o buffer redimensionável). Onde os bytes
//! moram muda junto com o buffer: depois de qualquer chamada ao JS, mede a vista de novo.
//!
//! shortcut: um agente só. `Atomics.wait` lança (como a thread principal de um navegador),
//! `notify` não acha ninguém e `waitAsync` não acorda por tempo. Upgrade: com workers.

use std::cmp::Ordering;

use crate::formas::{self, attr, Chave, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::grande::Grande;
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

use super::nucleo::{novo_iterador_array, IT_CHAVES, IT_PARES, IT_VALORES};

/// Tipos de item, na ordem da tabela [`TIPOS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tipo {
    I8,
    U8,
    U8c,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
    I64,
    U64,
    F16,
}

const TIPOS: [(Tipo, &str); 12] = [
    (Tipo::I8, "Int8Array"),
    (Tipo::U8, "Uint8Array"),
    (Tipo::U8c, "Uint8ClampedArray"),
    (Tipo::I16, "Int16Array"),
    (Tipo::U16, "Uint16Array"),
    (Tipo::I32, "Int32Array"),
    (Tipo::U32, "Uint32Array"),
    (Tipo::F32, "Float32Array"),
    (Tipo::F64, "Float64Array"),
    (Tipo::I64, "BigInt64Array"),
    (Tipo::U64, "BigUint64Array"),
    (Tipo::F16, "Float16Array"),
];

/// Um item cru: número, ou os 64 bits de um BigInt64/BigUint64.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Bruto {
    N(f64),
    B(u64),
}

impl Tipo {
    fn de(n: u32) -> Tipo {
        TIPOS[n as usize].0
    }

    fn numero(self) -> u32 {
        self as u32
    }

    fn tamanho(self) -> u32 {
        match self {
            Tipo::I8 | Tipo::U8 | Tipo::U8c => 1,
            Tipo::I16 | Tipo::U16 | Tipo::F16 => 2,
            Tipo::I32 | Tipo::U32 | Tipo::F32 => 4,
            Tipo::F64 | Tipo::I64 | Tipo::U64 => 8,
        }
    }

    fn nome(self) -> &'static str {
        TIPOS[self as usize].1
    }

    fn e_bigint(self) -> bool {
        matches!(self, Tipo::I64 | Tipo::U64)
    }

    /// Serve para Atomics (inteiro, sem o "clamped").
    fn atomico(self) -> bool {
        !matches!(self, Tipo::U8c | Tipo::F16 | Tipo::F32 | Tipo::F64)
    }

    /// Lê o item de `b` (little-endian salvo `be`).
    fn ler(self, b: &[u8], be: bool) -> Bruto {
        macro_rules! de {
            ($t:ty, $n:expr) => {{
                let mut x = [0u8; $n];
                x.copy_from_slice(&b[..$n]);
                if be {
                    <$t>::from_be_bytes(x)
                } else {
                    <$t>::from_le_bytes(x)
                }
            }};
        }
        match self {
            Tipo::I8 => Bruto::N(f64::from(b[0] as i8)),
            Tipo::U8 | Tipo::U8c => Bruto::N(f64::from(b[0])),
            Tipo::I16 => Bruto::N(f64::from(de!(i16, 2))),
            Tipo::U16 => Bruto::N(f64::from(de!(u16, 2))),
            Tipo::I32 => Bruto::N(f64::from(de!(i32, 4))),
            Tipo::U32 => Bruto::N(f64::from(de!(u32, 4))),
            Tipo::F16 => Bruto::N(f16_para_f64(de!(u16, 2))),
            Tipo::F32 => Bruto::N(f64::from(de!(f32, 4))),
            Tipo::F64 => Bruto::N(de!(f64, 8)),
            Tipo::I64 | Tipo::U64 => Bruto::B(de!(u64, 8)),
        }
    }

    /// Grava `x` convertido como a norma manda (ToInt8, ToUint8Clamp, ToBigInt64…).
    fn gravar(self, b: &mut [u8], x: Bruto, be: bool) {
        macro_rules! em {
            ($v:expr) => {{
                let v = $v;
                let bytes = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                b[..bytes.len()].copy_from_slice(&bytes);
            }};
        }
        let x = match x {
            Bruto::B(u) => return em!(u),
            Bruto::N(x) => x,
        };
        match self {
            Tipo::I8 | Tipo::U8 => b[0] = int32_cru(x) as u8,
            Tipo::U8c => {
                b[0] = if x.is_nan() || x <= 0.0 {
                    0
                } else if x >= 255.0 {
                    255
                } else {
                    meio_par(x) as u8
                }
            }
            Tipo::I16 | Tipo::U16 => em!(int32_cru(x) as u16),
            Tipo::I32 | Tipo::U32 => em!(int32_cru(x)),
            Tipo::F16 => em!(f64_para_f16(x)),
            Tipo::F32 => em!(x as f32),
            Tipo::F64 => em!(x),
            Tipo::I64 | Tipo::U64 => em!(0u64),
        }
    }
}

/// Os 32 bits de baixo de ToInt32/ToUint32.
fn int32_cru(x: f64) -> u32 {
    if x.is_finite() {
        x.trunc().rem_euclid(4_294_967_296.0) as u32
    } else {
        0
    }
}

/// Arredonda para o inteiro mais próximo; empate vai para o par.
fn meio_par(x: f64) -> f64 {
    let f = x.floor();
    let r = x - f;
    if r > 0.5 || (r == 0.5 && f % 2.0 != 0.0) {
        f + 1.0
    } else {
        f
    }
}

/// binary16 mais próximo (empate para o par), arredondando direto do f64.
fn f64_para_f16(x: f64) -> u16 {
    let sinal = if x.is_sign_negative() { 0x8000 } else { 0 };
    if x.is_nan() {
        return 0x7e00;
    }
    let a = x.abs();
    // Acima do meio entre 65504 e 65536 vira infinito.
    if a >= 65520.0 {
        return sinal | 0x7c00;
    }
    if a < 2f64.powi(-14) {
        // Subnormal: múltiplo de 2^-24 (a multiplicação por potência de 2 é exata).
        return sinal | meio_par(a * 2f64.powi(24)) as u16;
    }
    let mut e = ((a.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let mut m = meio_par((a / 2f64.powi(e) - 1.0) * 1024.0) as u16;
    if m == 1024 {
        m = 0;
        e += 1;
    }
    if e > 15 {
        return sinal | 0x7c00;
    }
    sinal | (((e + 15) as u16) << 10) | m
}

fn f16_para_f64(h: u16) -> f64 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = i32::from((h >> 10) & 0x1f);
    let m = f64::from(h & 0x3ff);
    s * match e {
        0 => m * 2f64.powi(-24),
        31 if m == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + m / 1024.0) * 2f64.powi(e - 15),
    }
}

/// Os 64 bits de baixo do BigInt (complemento de dois).
fn bits64(g: &Grande) -> u64 {
    let x = g.como_n_bits(64, false).unwrap_or_else(|_| Grande::zero());
    u64::from(x.mag.first().copied().unwrap_or(0)) | u64::from(x.mag.get(1).copied().unwrap_or(0)) << 32
}

// ---------------------------------------------------------------- buffers e vistas

/// Tamanho guardado no lugar do comprimento quando a vista acompanha o buffer.
const AUTO: u32 = u32::MAX;
// Flags do ArrayBuffer.
const REDIMENSIONAVEL: u32 = 1;
const COMPARTILHADO: u32 = 2;

fn classe(m: &Motor, v: Valor) -> Option<Classe> {
    m.e_objeto(v).then(|| m.formas.get(m.forma(v)).classe)
}

fn e_buffer(m: &Motor, v: Valor) -> bool {
    classe(m, v) == Some(Classe::ArrayBuffer)
}

fn flags(m: &Motor, buf: Valor) -> u32 {
    m.interno(buf, 1)
}

fn compartilhado(m: &Motor, buf: Valor) -> bool {
    flags(m, buf) & COMPARTILHADO != 0
}

fn redimensionavel(m: &Motor, buf: Valor) -> bool {
    flags(m, buf) & REDIMENSIONAVEL != 0
}

fn desligado(m: &Motor, buf: Valor) -> bool {
    Valor(m.interno(buf, 0)) == Valor::UNDEFINED
}

/// Onde mora o primeiro byte do buffer na gaiola e quantos são (desligado: `(0, 0)`).
fn bytes_do_buffer(m: &Motor, buf: Valor) -> (u32, u32) {
    let b = Valor(m.interno(buf, 0));
    if b == Valor::UNDEFINED {
        return (0, 0);
    }
    let d = b.deslocamento();
    (d + 8, m.g.u32_em(d + 4))
}

fn buf_len(m: &Motor, buf: Valor) -> u32 {
    bytes_do_buffer(m, buf).1
}

/// Bloco `BYTES` novo, zerado.
fn novos_bytes(m: &mut Motor, n: u32) -> Valor {
    let d = m.g.alocar(8 + n);
    m.g.por_u32(d, formas::BYTES.0);
    m.g.por_u32(d + 4, n);
    m.g.bytes_mut(d + 8, n).fill(0);
    Valor::de_deslocamento(d)
}

fn copiar_bytes(m: &mut Motor, de: u32, para: u32, n: u32) {
    let v = m.g.bytes(de, n).to_vec();
    m.g.bytes_mut(para, n).copy_from_slice(&v);
}

/// Troca o bloco do buffer por um de `n` bytes, com o começo do antigo.
fn realocar(m: &mut Motor, buf: Valor, n: u32) -> Res<()> {
    if !m.g.reservar(u64::from(n) + 8) {
        return m.erro_alcance("Array buffer allocation failed");
    }
    let novo = novos_bytes(m, n);
    let (de, velho) = bytes_do_buffer(m, buf);
    let para = novo.deslocamento() + 8;
    copiar_bytes(m, de, para, velho.min(n));
    m.por_interno_valor(buf, 0, novo);
    Ok(())
}

/// Começo (na gaiola) e comprimento atual de uma vista com itens de `tam` bytes;
/// None se o buffer foi desligado ou se a vista ficou fora dele (IsTypedArrayOutOfBounds).
fn medir(m: &Motor, o: Valor, tam: u32) -> Option<(u32, u32)> {
    let buf = Valor(m.interno(o, 0));
    if desligado(m, buf) {
        return None;
    }
    let (d, total) = bytes_do_buffer(m, buf);
    let (desl, n) = (m.interno(o, 2), m.interno(o, 3));
    if desl > total {
        return None;
    }
    if n == AUTO {
        return Some((d + desl, (total - desl) / tam));
    }
    (u64::from(desl) + u64::from(n) * u64::from(tam) <= u64::from(total)).then_some((d + desl, n))
}

fn tipo_de(m: &Motor, o: Valor) -> Tipo {
    Tipo::de(m.interno(o, 1))
}

/// Tipo, início do item 0 na gaiola e comprimento atual de um array tipado.
fn vista(m: &Motor, o: Valor) -> Option<(Tipo, u32, u32)> {
    let t = tipo_de(m, o);
    medir(m, o, t.tamanho()).map(|(d, n)| (t, d, n))
}

/// ToNumber ou ToBigInt, conforme o tipo.
fn converter(m: &mut Motor, t: Tipo, v: Valor) -> Res<Bruto> {
    if t.e_bigint() {
        let g = m.para_bigint(v)?;
        return Ok(Bruto::B(bits64(&g)));
    }
    Ok(Bruto::N(if v.e_int() { f64::from(v.como_int()) } else { m.para_numero(v)? }))
}

fn valor_de(m: &mut Motor, t: Tipo, x: Bruto) -> Valor {
    match x {
        Bruto::N(x) => match t {
            Tipo::I8 | Tipo::U8 | Tipo::U8c | Tipo::I16 | Tipo::U16 => Valor::int(x as i32),
            _ => m.numero(x),
        },
        Bruto::B(u) => {
            let g = if t == Tipo::I64 { Grande::de_i64(u as i64) } else { Grande::novo(false, vec![u as u32, (u >> 32) as u32]) };
            m.novo_bigint(&g)
        }
    }
}

/// Item cru `i` (já conferido contra o comprimento).
fn ler_cru(m: &Motor, t: Tipo, d: u32, i: u32) -> Bruto {
    let tam = t.tamanho();
    t.ler(m.g.bytes(d + i * tam, tam), false)
}

fn gravar_cru(m: &mut Motor, t: Tipo, d: u32, i: u32, x: Bruto) {
    let tam = t.tamanho();
    t.gravar(m.g.bytes_mut(d + i * tam, tam), x, false);
}

impl Motor {
    /// Comprimento atual do array tipado; None se desligado ou fora dos limites do buffer.
    pub(crate) fn tipado_medir(&self, o: Valor) -> Option<u32> {
        vista(self, o).map(|v| v.2)
    }

    /// Item `i` de um array tipado (None = índice inválido).
    pub(crate) fn tipado_ler(&mut self, o: Valor, i: u32) -> Option<Valor> {
        let (t, d, n) = vista(self, o)?;
        if i >= n {
            return None;
        }
        let x = ler_cru(self, t, d, i);
        Some(valor_de(self, t, x))
    }

    /// TypedArraySetElement: converte primeiro; índice inválido (depois disso) não faz nada.
    pub(crate) fn tipado_gravar(&mut self, o: Valor, i: u32, v: Valor) -> Res<()> {
        let x = converter(self, tipo_de(self, o), v)?;
        if let Some((t, d, n)) = vista(self, o) {
            if i < n {
                gravar_cru(self, t, d, i, x);
            }
        }
        Ok(())
    }

    pub(crate) fn tipado_len(&self, o: Valor) -> u32 {
        self.tipado_medir(o).unwrap_or(0)
    }

    /// Chave numérica de array tipado: `Some(Some(i))` índice, `Some(None)` número que nunca é
    /// índice válido ("-0", "1.5"…), `None` chave comum.
    pub(crate) fn tipado_chave(&self, _o: Valor, k: ChaveProp) -> Option<Option<u32>> {
        match k {
            ChaveProp::Indice(i) => Some(Some(i)),
            ChaveProp::Nome(c) => self.numerico_canonico(c).then_some(None),
        }
    }

    /// CanonicalNumericIndexString dá número: num array tipado a chave nunca é propriedade comum.
    /// (Os índices de array já chegam como `ChaveProp::Indice`.)
    pub(crate) fn numerico_canonico(&self, c: Chave) -> bool {
        if c.e_simbolo() {
            return false;
        }
        let t = self.nomes.texto(c);
        match t.first() {
            Some(&ch) if (u16::from(b'0')..=u16::from(b'9')).contains(&ch) || ch == u16::from(b'-') || ch == u16::from(b'I') || ch == u16::from(b'N') => {}
            _ => return false,
        }
        if t == [u16::from(b'-'), u16::from(b'0')] {
            return true;
        }
        let x = crate::numeros::texto_para_numero(t);
        let mut s: Vec<u16> = Vec::new();
        crate::numeros::escrever_numero(x, &mut |b| s.extend(b.iter().map(|&c| u16::from(c))));
        s == t
    }

    /// DetachArrayBuffer (o `$262.detachArrayBuffer` dos testes, e `transfer`).
    pub fn desligar_array_buffer(&mut self, buf: Valor) -> Res<()> {
        if !e_buffer(self, buf) || compartilhado(self, buf) {
            return self.erro_tipo("not an ArrayBuffer");
        }
        self.por_interno_valor(buf, 0, Valor::UNDEFINED);
        Ok(())
    }
}

// ---------------------------------------------------------------- instalar

pub(super) fn instalar(m: &mut Motor) {
    let op = m.intr.objeto_proto;
    let g = m.intr.global;

    // ---- ArrayBuffer
    let abp = proto_com(m, op);
    let ab = m.nativa("ArrayBuffer", 1, ab_construtor, true);
    ligar(m, ab, "ArrayBuffer", abp);
    super::ligar_especie(m, ab);
    m.metodo(ab, "isView", 1, ab_is_view);
    getter(m, abp, "byteLength", "get byteLength", ab_byte_length);
    getter(m, abp, "maxByteLength", "get maxByteLength", ab_max_byte_length);
    getter(m, abp, "resizable", "get resizable", ab_resizable);
    getter(m, abp, "detached", "get detached", ab_detached);
    metodo_com(m, abp, "slice", 2, ab_slice, ab);
    m.metodo(abp, "resize", 1, ab_resize);
    m.metodo(abp, "transfer", 0, ab_transfer);
    m.metodo(abp, "transferToFixedLength", 0, ab_transfer_fixo);
    etiqueta(m, abp, "ArrayBuffer");
    m.intr.array_buffer_proto = abp;

    // ---- SharedArrayBuffer
    let sbp = proto_com(m, op);
    let sb = m.nativa("SharedArrayBuffer", 1, sab_construtor, true);
    ligar(m, sb, "SharedArrayBuffer", sbp);
    super::ligar_especie(m, sb);
    getter(m, sbp, "byteLength", "get byteLength", sab_byte_length);
    getter(m, sbp, "growable", "get growable", sab_growable);
    getter(m, sbp, "maxByteLength", "get maxByteLength", sab_max_byte_length);
    m.metodo(sbp, "grow", 1, sab_grow);
    metodo_com(m, sbp, "slice", 2, sab_slice, sb);
    etiqueta(m, sbp, "SharedArrayBuffer");
    m.intr.shared_array_buffer_proto = sbp;

    // ---- %TypedArray%
    let tap = proto_com(m, op);
    let ta = m.nativa("TypedArray", 0, ta_abstrato, true);
    super::ligar_especie(m, ta);
    m.definir_propria(ta, fixos::PROTOTYPE, tap, 0);
    m.definir_propria(tap, fixos::CONSTRUCTOR, ta, attr::METODO);
    m.metodo(ta, "from", 1, ta_from);
    m.metodo(ta, "of", 0, ta_of);
    getter(m, tap, "buffer", "get buffer", ta_buffer);
    getter(m, tap, "byteLength", "get byteLength", ta_byte_length);
    getter(m, tap, "byteOffset", "get byteOffset", ta_byte_offset);
    getter(m, tap, "length", "get length", ta_length);
    let tag = m.nativa("get [Symbol.toStringTag]", 0, ta_tag, false);
    m.definir_acessor(tap, ChaveProp::Nome(simb::TO_STRING_TAG), tag, false, false);
    let proprios: &[(&'static str, u32, FnNativa)] = &[
        ("at", 1, ta_at),
        ("copyWithin", 2, ta_copy_within),
        ("entries", 0, ta_entries),
        ("every", 1, ta_every),
        ("fill", 1, ta_fill),
        ("filter", 1, ta_filter),
        ("find", 1, ta_find),
        ("findIndex", 1, ta_find_index),
        ("findLast", 1, ta_find_last),
        ("findLastIndex", 1, ta_find_last_index),
        ("forEach", 1, ta_for_each),
        ("includes", 1, ta_includes),
        ("indexOf", 1, ta_index_of),
        ("join", 1, ta_join),
        ("keys", 0, ta_keys),
        ("lastIndexOf", 1, ta_last_index_of),
        ("map", 1, ta_map),
        ("reduce", 1, ta_reduce),
        ("reduceRight", 1, ta_reduce_right),
        ("reverse", 0, ta_reverse),
        ("set", 1, ta_set),
        ("slice", 2, ta_slice),
        ("some", 1, ta_some),
        ("sort", 1, ta_sort),
        ("subarray", 2, ta_subarray),
        ("toLocaleString", 0, ta_to_locale_string),
        ("toReversed", 0, ta_to_reversed),
        ("toSorted", 1, ta_to_sorted),
        ("with", 2, ta_with),
    ];
    for &(nome, n, f) in proprios {
        m.metodo(tap, nome, n, f);
    }
    let valores = m.metodo(tap, "values", 0, ta_values);
    m.definir_propria(tap, simb::ITERATOR, valores, attr::METODO);
    // A norma pede o mesmo objeto do Array.prototype.toString.
    let ap = m.intr.array_proto;
    let ts = m.obter(ap, ChaveProp::Nome(fixos::TO_STRING)).unwrap_or(Valor::UNDEFINED);
    m.definir_propria(tap, fixos::TO_STRING, ts, attr::METODO);

    // ---- Uint8Array & cia.
    let id = m.registrar_nativa("TypedArray", ta_construtor, true);
    for (t, nome) in TIPOS {
        let p = proto_com(m, tap);
        let c = m.nativa_com(id, 3, Valor::int(t.numero() as i32));
        let n = m.texto(nome);
        m.definir_propria(c, fixos::NAME, n, attr::CONFIGURAVEL);
        m.definir_proto(c, ta);
        let bpe = Valor::int(t.tamanho() as i32);
        let k = m.chave("BYTES_PER_ELEMENT");
        m.definir_propria(c, k, bpe, 0);
        m.definir_propria(p, k, bpe, 0);
        ligar(m, c, nome, p);
        m.intr.tipados_protos[t as usize] = p;
        m.intr.tipados_ctors[t as usize] = c;
    }

    // ---- DataView
    let dvp = proto_com(m, op);
    let dv = m.nativa("DataView", 1, dv_construtor, true);
    ligar(m, dv, "DataView", dvp);
    getter(m, dvp, "buffer", "get buffer", dv_buffer);
    getter(m, dvp, "byteLength", "get byteLength", dv_byte_length);
    getter(m, dvp, "byteOffset", "get byteOffset", dv_byte_offset);
    let ler = m.registrar_nativa("get", dv_get, false);
    let gravar = m.registrar_nativa("set", dv_set, false);
    for (t, nome) in TIPOS {
        if t == Tipo::U8c {
            continue;
        }
        let base = &nome[..nome.len() - "Array".len()];
        for (pre, id, n) in [("get", ler, 1), ("set", gravar, 2)] {
            let f = m.nativa_com(id, n, Valor::int(t.numero() as i32));
            let nome_f = m.texto(&format!("{pre}{base}"));
            m.definir_propria(f, fixos::NAME, nome_f, attr::CONFIGURAVEL);
            let c = m.chave(&format!("{pre}{base}"));
            m.definir_propria(dvp, c, f, attr::METODO);
        }
    }
    etiqueta(m, dvp, "DataView");

    // ---- Atomics
    let at = proto_com(m, op);
    let atomicos: &[(&'static str, u32, FnNativa)] = &[
        ("add", 3, at_add),
        ("and", 3, at_and),
        ("compareExchange", 4, at_compare_exchange),
        ("exchange", 3, at_exchange),
        ("isLockFree", 1, at_is_lock_free),
        ("load", 2, at_load),
        ("notify", 3, at_notify),
        ("or", 3, at_or),
        ("pause", 0, at_pause),
        ("store", 3, at_store),
        ("sub", 3, at_sub),
        ("wait", 4, at_wait),
        ("waitAsync", 4, at_wait_async),
        ("xor", 3, at_xor),
    ];
    for &(nome, n, f) in atomicos {
        m.metodo(at, nome, n, f);
    }
    etiqueta(m, at, "Atomics");
    let k = m.chave("Atomics");
    m.definir_propria(g, k, at, attr::METODO);

    // ---- Math.f16round
    let km = m.chave("Math");
    if let Ok(math) = m.obter(g, ChaveProp::Nome(km)) {
        if m.e_objeto(math) {
            m.metodo(math, "f16round", 1, f16round);
        }
    }
}

fn proto_com(m: &mut Motor, pai: Valor) -> Valor {
    let f = m.forma_para(Classe::Comum, pai, 0);
    m.novo_objeto(f)
}

fn ligar(m: &mut Motor, c: Valor, nome: &str, proto: Valor) {
    m.definir_propria(c, fixos::PROTOTYPE, proto, 0);
    m.definir_propria(proto, fixos::CONSTRUCTOR, c, attr::METODO);
    let k = m.chave(nome);
    let g = m.intr.global;
    m.definir_propria(g, k, c, attr::METODO);
}

fn getter(m: &mut Motor, obj: Valor, nome: &'static str, nome_get: &'static str, f: FnNativa) {
    let g = m.nativa(nome_get, 0, f, false);
    let c = m.chave(nome);
    m.definir_acessor(obj, ChaveProp::Nome(c), g, false, false);
}

/// Método que captura `v` (o construtor original, para a espécie padrão).
fn metodo_com(m: &mut Motor, obj: Valor, nome: &'static str, n: u32, f: FnNativa, v: Valor) {
    let id = m.registrar_nativa(nome, f, false);
    let func = m.nativa_com(id, n, v);
    let t = m.texto(nome);
    m.definir_propria(func, fixos::NAME, t, attr::CONFIGURAVEL);
    let c = m.chave(nome);
    m.definir_propria(obj, c, func, attr::METODO);
}

fn etiqueta(m: &mut Motor, obj: Valor, nome: &str) {
    let t = m.texto(nome);
    m.definir_propria(obj, simb::TO_STRING_TAG, t, attr::CONFIGURAVEL);
}

/// Protótipo de `new X()`: o `prototype` de `nt`, ou `padrao`.
fn proto_de_nt(m: &mut Motor, nt: Valor, padrao: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return Ok(padrao);
    }
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    Ok(if m.e_objeto(p) { p } else { padrao })
}

/// ToIndex (§7.1.22); acima do que a gaiola guarda também é RangeError.
fn para_indice(m: &mut Motor, v: Valor, oque: &str) -> Res<u32> {
    if v == Valor::UNDEFINED {
        return Ok(0);
    }
    let x = m.para_inteiro(v)?;
    if !(0.0..=f64::from(u32::MAX - 16)).contains(&x) {
        return m.erro_alcance(&format!("Invalid {oque}"));
    }
    Ok(x as u32)
}

/// Início/fim relativos (`slice(-2)`), já presos em `0..=n`.
fn relativo(m: &mut Motor, v: Valor, n: u32, padrao: u32) -> Res<u32> {
    if v == Valor::UNDEFINED {
        return Ok(padrao);
    }
    let x = m.para_inteiro(v)?;
    let n = f64::from(n);
    Ok((if x < 0.0 { (n + x).max(0.0) } else { x.min(n) }) as u32)
}

/// SpeciesConstructor(o, padrao).
fn especie(m: &mut Motor, o: Valor, padrao: Valor) -> Res<Valor> {
    let c = m.obter(o, ChaveProp::Nome(fixos::CONSTRUCTOR))?;
    if c == Valor::UNDEFINED {
        return Ok(padrao);
    }
    if !m.e_objeto(c) {
        return m.erro_tipo("object.constructor is not an object");
    }
    let sp = m.obter(c, ChaveProp::Nome(simb::SPECIES))?;
    if sp.e_nulo_ou_indefinido() {
        return Ok(padrao);
    }
    if m.e_construtor(sp) {
        return Ok(sp);
    }
    m.erro_tipo("object.constructor[Symbol.species] is not a constructor")
}

fn funcao(m: &mut Motor, f: Valor) -> Res<()> {
    if m.e_funcao(f) {
        return Ok(());
    }
    m.erro_tipo("callback is not a function")
}

/// Os itens de um iterador obtido pelo método `metodo` de `x` (IteratorToList).
fn lista_do_iterador(m: &mut Motor, x: Valor, metodo: Valor) -> Res<Vec<Valor>> {
    let it = m.chamar(metodo, x, &[])?;
    if !m.e_objeto(it) {
        return m.erro_tipo("Result of the Symbol.iterator method is not an object");
    }
    let next = m.obter(it, ChaveProp::Nome(fixos::NEXT))?;
    let mut v = Vec::new();
    loop {
        let r = m.chamar(next, it, &[])?;
        if !m.e_objeto(r) {
            return m.erro_tipo("Iterator result is not an object");
        }
        let feito = m.obter(r, ChaveProp::Nome(fixos::DONE))?;
        if m.verdadeiro(feito) {
            return Ok(v);
        }
        v.push(m.obter(r, ChaveProp::Nome(fixos::VALUE))?);
    }
}

// ---------------------------------------------------------------- ArrayBuffer

fn novo_buffer(m: &mut Motor, n: u32, proto: Valor, fl: u32, max: u32) -> Res<Valor> {
    if !m.g.reservar(u64::from(n) + 8) {
        return m.erro_alcance("Array buffer allocation failed");
    }
    let f = m.forma_para(Classe::ArrayBuffer, proto, 0);
    let o = m.novo_objeto(f);
    let b = novos_bytes(m, n);
    m.por_interno_valor(o, 0, b);
    m.por_interno(o, 1, fl);
    m.por_interno(o, 2, max);
    Ok(o)
}

/// GetArrayBufferMaxByteLengthOption.
fn opcao_max(m: &mut Motor, o: Valor) -> Res<Option<f64>> {
    if !m.e_objeto(o) {
        return Ok(None);
    }
    let k = m.chave("maxByteLength");
    let v = m.obter(o, ChaveProp::Nome(k))?;
    if v == Valor::UNDEFINED {
        return Ok(None);
    }
    para_indice_53(m, v).map(Some)
}

/// ToIndex sem o limite da gaiola (até 2^53 - 1).
fn para_indice_53(m: &mut Motor, v: Valor) -> Res<f64> {
    let x = m.para_inteiro(v)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&x) {
        return m.erro_alcance("Invalid array buffer length");
    }
    Ok(x)
}

/// Os construtores de ArrayBuffer e SharedArrayBuffer.
fn construir_buffer(m: &mut Motor, a: Args, nt: Valor, nome: &str, fl: u32, padrao: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo(&format!("Constructor {nome} requires 'new'"));
    }
    let n = para_indice_53(m, m.arg(a, 0))?;
    let max = opcao_max(m, m.arg(a, 1))?;
    if max.is_some_and(|x| n > x) {
        return m.erro_alcance("byteLength exceeds maxByteLength");
    }
    let p = proto_de_nt(m, nt, padrao)?;
    // O que não cabe na gaiola só falha depois de pegar o protótipo, como a alocação da norma.
    let limite = f64::from(u32::MAX - 16);
    if n > limite || max.is_some_and(|x| x > limite) {
        return m.erro_alcance("Array buffer allocation failed");
    }
    let fl = if max.is_some() { fl | REDIMENSIONAVEL } else { fl };
    novo_buffer(m, n as u32, p, fl, max.unwrap_or(n) as u32)
}

fn ab_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let p = m.intr.array_buffer_proto;
    construir_buffer(m, a, nt, "ArrayBuffer", 0, p)
}

fn sab_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let p = m.intr.shared_array_buffer_proto;
    construir_buffer(m, a, nt, "SharedArrayBuffer", COMPARTILHADO, p)
}

/// `this` como ArrayBuffer (`compart` = false) ou SharedArrayBuffer.
fn buffer_de(m: &mut Motor, this: Valor, nome: &str, compart: bool) -> Res<Valor> {
    if e_buffer(m, this) && compartilhado(m, this) == compart {
        return Ok(this);
    }
    let c = if compart { "SharedArrayBuffer" } else { "ArrayBuffer" };
    m.erro_tipo(&format!("Method {c}.prototype.{nome} called on incompatible receiver"))
}

fn ab_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "byteLength", false)?;
    Ok(m.numero(f64::from(buf_len(m, b))))
}

fn ab_max_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "maxByteLength", false)?;
    let n = if desligado(m, b) {
        0
    } else if redimensionavel(m, b) {
        m.interno(b, 2)
    } else {
        buf_len(m, b)
    };
    Ok(m.numero(f64::from(n)))
}

fn ab_resizable(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "resizable", false)?;
    Ok(Valor::bool(redimensionavel(m, b)))
}

fn ab_detached(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "detached", false)?;
    Ok(Valor::bool(desligado(m, b)))
}

fn ab_is_view(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    Ok(Valor::bool(matches!(classe(m, v), Some(Classe::ArrayTipado | Classe::DataView))))
}

/// `slice` dos dois buffers (o construtor original vem capturado).
fn fatiar_buffer(m: &mut Motor, this: Valor, a: Args, compart: bool) -> Res<Valor> {
    let b = buffer_de(m, this, "slice", compart)?;
    if desligado(m, b) {
        return m.erro_tipo("Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer");
    }
    let n = buf_len(m, b);
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fim = relativo(m, m.arg(a, 1), n, n)?;
    let novo_n = fim.saturating_sub(ini);
    let padrao = m.capturado();
    let c = especie(m, b, padrao)?;
    let nv = m.numero(f64::from(novo_n));
    let novo = m.construir(c, &[nv], c)?;
    if !e_buffer(m, novo) || compartilhado(m, novo) != compart || desligado(m, novo) {
        return m.erro_tipo("Species constructor did not return a usable buffer");
    }
    if novo == b {
        return m.erro_tipo("Species constructor returned the same buffer");
    }
    if buf_len(m, novo) < novo_n {
        return m.erro_tipo("Species constructor returned a buffer that is too small");
    }
    if desligado(m, b) {
        return m.erro_tipo("Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer");
    }
    let atual = buf_len(m, b);
    if ini < atual {
        let cont = novo_n.min(atual - ini);
        let (de, _) = bytes_do_buffer(m, b);
        let (para, _) = bytes_do_buffer(m, novo);
        copiar_bytes(m, de + ini, para, cont);
    }
    Ok(novo)
}

fn ab_slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    fatiar_buffer(m, this, a, false)
}

fn ab_resize(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "resize", false)?;
    if !redimensionavel(m, b) {
        return m.erro_tipo("Method ArrayBuffer.prototype.resize called on a fixed-length buffer");
    }
    let n = para_indice(m, m.arg(a, 0), "length")?;
    if desligado(m, b) {
        return m.erro_tipo("Cannot resize a detached ArrayBuffer");
    }
    if n > m.interno(b, 2) {
        return m.erro_alcance("ArrayBuffer.prototype.resize: Invalid length parameter");
    }
    realocar(m, b, n)?;
    Ok(Valor::UNDEFINED)
}

/// ArrayBufferCopyAndDetach.
fn transferir(m: &mut Motor, this: Valor, a: Args, preservar: bool) -> Res<Valor> {
    let b = buffer_de(m, this, "transfer", false)?;
    let lv = m.arg(a, 0);
    let n = if lv == Valor::UNDEFINED { buf_len(m, b) } else { para_indice(m, lv, "length")? };
    if desligado(m, b) {
        return m.erro_tipo("Cannot transfer a detached ArrayBuffer");
    }
    let (fl, max) = if preservar && redimensionavel(m, b) { (REDIMENSIONAVEL, m.interno(b, 2)) } else { (0, n) };
    if n > max {
        return m.erro_alcance("ArrayBuffer.prototype.transfer: Invalid length parameter");
    }
    let p = m.intr.array_buffer_proto;
    let novo = novo_buffer(m, n, p, fl, max)?;
    let (de, velho) = bytes_do_buffer(m, b);
    let (para, _) = bytes_do_buffer(m, novo);
    copiar_bytes(m, de, para, velho.min(n));
    m.por_interno_valor(b, 0, Valor::UNDEFINED);
    Ok(novo)
}

fn ab_transfer(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    transferir(m, this, a, true)
}

fn ab_transfer_fixo(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    transferir(m, this, a, false)
}

// ---------------------------------------------------------------- SharedArrayBuffer

fn sab_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "byteLength", true)?;
    Ok(m.numero(f64::from(buf_len(m, b))))
}

fn sab_growable(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "growable", true)?;
    Ok(Valor::bool(redimensionavel(m, b)))
}

fn sab_max_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "maxByteLength", true)?;
    let n = if redimensionavel(m, b) { m.interno(b, 2) } else { buf_len(m, b) };
    Ok(m.numero(f64::from(n)))
}

fn sab_grow(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "grow", true)?;
    if !redimensionavel(m, b) {
        return m.erro_tipo("Method SharedArrayBuffer.prototype.grow called on a fixed-length buffer");
    }
    let n = para_indice(m, m.arg(a, 0), "length")?;
    if n < buf_len(m, b) || n > m.interno(b, 2) {
        return m.erro_alcance("SharedArrayBuffer.prototype.grow: Invalid length parameter");
    }
    realocar(m, b, n)?;
    Ok(Valor::UNDEFINED)
}

fn sab_slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    fatiar_buffer(m, this, a, true)
}

// ---------------------------------------------------------------- arrays tipados: criação

fn ta_abstrato(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    m.erro_tipo("Abstract class TypedArray not directly constructable")
}

/// Array tipado novo sobre `buf` (sem conferir limites).
fn novo_tipado(m: &mut Motor, t: Tipo, buf: Valor, deslocamento: u32, n: u32, proto: Valor) -> Valor {
    let f = m.forma_para(Classe::ArrayTipado, proto, 0);
    let o = m.novo_objeto(f);
    m.por_interno_valor(o, 0, buf);
    m.por_interno(o, 1, t.numero());
    m.por_interno(o, 2, deslocamento);
    m.por_interno(o, 3, n);
    o
}

/// Array tipado zerado com `n` itens num buffer só dele.
fn tipado_com_proto(m: &mut Motor, t: Tipo, n: u32, proto: Valor) -> Res<Valor> {
    let bytes = n.checked_mul(t.tamanho()).filter(|&b| b < u32::MAX - 16);
    let Some(bytes) = bytes else { return m.erro_alcance("Invalid typed array length") };
    let abp = m.intr.array_buffer_proto;
    let buf = novo_buffer(m, bytes, abp, 0, bytes)?;
    Ok(novo_tipado(m, t, buf, 0, n, proto))
}

/// TypedArrayCreateSameType: o construtor original do tipo, sem passar pelo JS.
fn tipado_vazio(m: &mut Motor, t: Tipo, n: u32) -> Res<Valor> {
    let p = m.intr.tipados_protos[t as usize];
    tipado_com_proto(m, t, n, p)
}

fn ta_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let t = Tipo::de(m.capturado().como_int() as u32);
    if nt == Valor::UNDEFINED {
        return m.erro_tipo(&format!("Constructor {} requires 'new'", t.nome()));
    }
    let padrao = m.intr.tipados_protos[t as usize];
    let x = m.arg(a, 0);
    if !m.e_objeto(x) {
        let n = para_indice(m, x, "typed array length")?;
        let p = proto_de_nt(m, nt, padrao)?;
        return tipado_com_proto(m, t, n, p);
    }
    let p = proto_de_nt(m, nt, padrao)?;
    match classe(m, x) {
        Some(Classe::ArrayTipado) => {
            let Some((ts, _, n)) = vista(m, x) else {
                return m.erro_tipo("Cannot construct from a detached or out-of-bounds TypedArray");
            };
            let o = tipado_com_proto(m, t, n, p)?;
            if ts.e_bigint() != t.e_bigint() {
                return m.erro_tipo(&format!("Cannot mix BigInt and other types ({} from {})", t.nome(), ts.nome()));
            }
            let (_, de, _) = vista(m, x).unwrap();
            let (_, para, _) = vista(m, o).unwrap();
            if ts == t {
                copiar_bytes(m, de, para, n * t.tamanho());
            } else {
                for i in 0..n {
                    let v = ler_cru(m, ts, de, i);
                    gravar_cru(m, t, para, i, v);
                }
            }
            Ok(o)
        }
        Some(Classe::ArrayBuffer) => do_buffer(m, t, x, a, p),
        _ => {
            if let Some(it) = m.obter_metodo(x, ChaveProp::Nome(simb::ITERATOR))? {
                let itens = lista_do_iterador(m, x, it)?;
                let o = tipado_com_proto(m, t, itens.len() as u32, p)?;
                for (i, v) in itens.into_iter().enumerate() {
                    m.tipado_gravar(o, i as u32, v)?;
                }
                return Ok(o);
            }
            let n = m.comprimento_de(x)?;
            if n > u64::from(u32::MAX) {
                return m.erro_alcance("Invalid typed array length");
            }
            let o = tipado_com_proto(m, t, n as u32, p)?;
            for i in 0..n {
                let v = m.obter_indice(x, i)?;
                m.tipado_gravar(o, i as u32, v)?;
            }
            Ok(o)
        }
    }
}

/// InitializeTypedArrayFromArrayBuffer.
fn do_buffer(m: &mut Motor, t: Tipo, buf: Valor, a: Args, p: Valor) -> Res<Valor> {
    let tam = t.tamanho();
    let desl = para_indice(m, m.arg(a, 1), "offset")?;
    if desl % tam != 0 {
        return m.erro_alcance(&format!("start offset of {} should be a multiple of {tam}", t.nome()));
    }
    let lv = m.arg(a, 2);
    let pedido = if lv == Valor::UNDEFINED { None } else { Some(para_indice(m, lv, "typed array length")?) };
    if desligado(m, buf) {
        return m.erro_tipo("Cannot construct a TypedArray on a detached ArrayBuffer");
    }
    let total = buf_len(m, buf);
    if pedido.is_none() && redimensionavel(m, buf) {
        if desl > total {
            return m.erro_alcance(&format!("Start offset {desl} is outside the bounds of the buffer"));
        }
        return Ok(novo_tipado(m, t, buf, desl, AUTO, p));
    }
    let n = match pedido {
        None => {
            if total % tam != 0 {
                return m.erro_alcance(&format!("byte length of {} should be a multiple of {tam}", t.nome()));
            }
            if desl > total {
                return m.erro_alcance(&format!("Start offset {desl} is outside the bounds of the buffer"));
            }
            (total - desl) / tam
        }
        Some(n) => {
            if u64::from(desl) + u64::from(n) * u64::from(tam) > u64::from(total) {
                return m.erro_alcance(&format!("Invalid typed array length: {n}"));
            }
            n
        }
    };
    Ok(novo_tipado(m, t, buf, desl, n, p))
}

/// Índice do tipo se `c` é um dos construtores originais.
fn tipo_original(m: &Motor, c: Valor) -> Option<Tipo> {
    m.intr.tipados_ctors.iter().position(|&x| x == c).map(|i| TIPOS[i].0)
}

/// TypedArrayCreateFromConstructor.
fn criar_de_construtor(m: &mut Motor, c: Valor, args: &[Valor]) -> Res<Valor> {
    // O construtor original com só o comprimento não tem nada a observar: direto.
    if let (Some(t), [n]) = (tipo_original(m, c), args) {
        if n.e_int() && n.como_int() >= 0 {
            return tipado_vazio(m, t, n.como_int() as u32);
        }
    }
    let novo = m.construir(c, args, c)?;
    let (_, n) = validar(m, novo, "TypedArray species")?;
    if let [x] = args {
        if m.e_numero(*x) && f64::from(n) < m.f64_de(*x) {
            return m.erro_tipo("TypedArray species constructor returned a too short array");
        }
    }
    Ok(novo)
}

/// TypedArraySpeciesCreate.
fn criar_especie(m: &mut Motor, exemplo: Valor, args: &[Valor]) -> Res<Valor> {
    let t = tipo_de(m, exemplo);
    let padrao = m.intr.tipados_ctors[t as usize];
    let c = especie(m, exemplo, padrao)?;
    let novo = criar_de_construtor(m, c, args)?;
    if tipo_de(m, novo).e_bigint() != t.e_bigint() {
        return m.erro_tipo("TypedArray species constructor returned an array with a different content type");
    }
    Ok(novo)
}

fn ta_from(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let c = this;
    if !m.e_construtor(c) {
        return m.erro_tipo("TypedArray.from: this is not a constructor");
    }
    let (x, f, this_f) = (m.arg(a, 0), m.arg(a, 1), m.arg(a, 2));
    let mapear = f != Valor::UNDEFINED;
    if mapear {
        funcao(m, f)?;
    }
    if let Some(it) = m.obter_metodo(x, ChaveProp::Nome(simb::ITERATOR))? {
        let itens = lista_do_iterador(m, x, it)?;
        let nv = m.numero(itens.len() as f64);
        let o = criar_de_construtor(m, c, &[nv])?;
        for (i, mut v) in itens.into_iter().enumerate() {
            if mapear {
                let iv = m.numero(i as f64);
                v = m.chamar(f, this_f, &[v, iv])?;
            }
            gravar_em(m, o, i as u32, v)?;
        }
        return Ok(o);
    }
    let fonte = m.para_objeto(x)?;
    let n = m.comprimento_de(fonte)?;
    let nv = m.numero(n as f64);
    let o = criar_de_construtor(m, c, &[nv])?;
    for i in 0..n {
        let mut v = m.obter_indice(fonte, i)?;
        if mapear {
            let iv = m.numero(i as f64);
            v = m.chamar(f, this_f, &[v, iv])?;
        }
        gravar_em(m, o, i as u32, v)?;
    }
    Ok(o)
}

fn ta_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_construtor(this) {
        return m.erro_tipo("TypedArray.of: this is not a constructor");
    }
    let nv = m.numero(f64::from(a.n));
    let o = criar_de_construtor(m, this, &[nv])?;
    for i in 0..a.n {
        let v = m.arg(a, i);
        gravar_em(m, o, i, v)?;
    }
    Ok(o)
}

/// `Set(o, i, v, true)` num objeto que já se sabe array tipado.
fn gravar_em(m: &mut Motor, o: Valor, i: u32, v: Valor) -> Res<()> {
    m.tipado_gravar(o, i, v)
}

// ---------------------------------------------------------------- arrays tipados: acesso

/// RequireInternalSlot([[TypedArrayName]]).
fn exigir_tipado(m: &mut Motor, this: Valor, nome: &str) -> Res<Tipo> {
    if classe(m, this) == Some(Classe::ArrayTipado) {
        return Ok(tipo_de(m, this));
    }
    m.erro_tipo(&format!("this is not a typed array ({nome})"))
}

/// ValidateTypedArray: tipo e comprimento atual; TypeError se desligado ou fora dos limites.
fn validar(m: &mut Motor, this: Valor, nome: &str) -> Res<(Tipo, u32)> {
    exigir_tipado(m, this, nome)?;
    match vista(m, this) {
        Some((t, _, n)) => Ok((t, n)),
        None => m.erro_tipo(&format!("Cannot perform {nome} on a detached or out-of-bounds TypedArray")),
    }
}

fn ler(m: &mut Motor, o: Valor, i: u32) -> Valor {
    m.tipado_ler(o, i).unwrap_or(Valor::UNDEFINED)
}

fn ta_buffer(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    exigir_tipado(m, this, "buffer")?;
    Ok(Valor(m.interno(this, 0)))
}

fn ta_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let t = exigir_tipado(m, this, "byteLength")?;
    let n = m.tipado_len(this);
    Ok(m.numero(f64::from(n) * f64::from(t.tamanho())))
}

fn ta_byte_offset(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    exigir_tipado(m, this, "byteOffset")?;
    let d = if vista(m, this).is_some() { m.interno(this, 2) } else { 0 };
    Ok(m.numero(f64::from(d)))
}

fn ta_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    exigir_tipado(m, this, "length")?;
    let n = m.tipado_len(this);
    Ok(m.numero(f64::from(n)))
}

fn ta_tag(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if classe(m, this) != Some(Classe::ArrayTipado) {
        return Ok(Valor::UNDEFINED);
    }
    let t = tipo_de(m, this);
    Ok(m.texto(t.nome()))
}

fn iterador(m: &mut Motor, this: Valor, nome: &str, tipo: u32) -> Res<Valor> {
    validar(m, this, nome)?;
    novo_iterador_array(m, this, tipo)
}

fn ta_values(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    iterador(m, this, "values", IT_VALORES)
}

fn ta_keys(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    iterador(m, this, "keys", IT_CHAVES)
}

fn ta_entries(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    iterador(m, this, "entries", IT_PARES)
}

fn ta_at(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "at")?;
    let rel = m.para_inteiro(m.arg(a, 0))?;
    let k = if rel >= 0.0 { rel } else { f64::from(n) + rel };
    if k < 0.0 || k >= f64::from(n) {
        return Ok(Valor::UNDEFINED);
    }
    Ok(ler(m, this, k as u32))
}

/// Chama `f(item, i, this)` em ordem (ou de trás para frente) até `chega` aceitar o
/// resultado; devolve onde parou.
fn percorrer(m: &mut Motor, this: Valor, a: Args, nome: &str, reverso: bool, chega: fn(&mut Motor, Valor) -> bool) -> Res<Option<(u32, Valor)>> {
    let (_, n) = validar(m, this, nome)?;
    let (f, t) = (m.arg(a, 0), m.arg(a, 1));
    funcao(m, f)?;
    for j in 0..n {
        let i = if reverso { n - 1 - j } else { j };
        let v = ler(m, this, i);
        let iv = m.numero(f64::from(i));
        let r = m.chamar(f, t, &[v, iv, this])?;
        if chega(m, r) {
            return Ok(Some((i, v)));
        }
    }
    Ok(None)
}

fn verdade(m: &mut Motor, r: Valor) -> bool {
    m.verdadeiro(r)
}

fn mentira(m: &mut Motor, r: Valor) -> bool {
    !m.verdadeiro(r)
}

fn nunca(_: &mut Motor, _: Valor) -> bool {
    false
}

fn ta_every(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(percorrer(m, this, a, "every", false, mentira)?.is_none()))
}

fn ta_some(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(Valor::bool(percorrer(m, this, a, "some", false, verdade)?.is_some()))
}

fn ta_for_each(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    percorrer(m, this, a, "forEach", false, nunca)?;
    Ok(Valor::UNDEFINED)
}

fn ta_find(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(percorrer(m, this, a, "find", false, verdade)?.map_or(Valor::UNDEFINED, |(_, v)| v))
}

fn ta_find_last(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(percorrer(m, this, a, "findLast", true, verdade)?.map_or(Valor::UNDEFINED, |(_, v)| v))
}

fn ta_find_index(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(percorrer(m, this, a, "findIndex", false, verdade)?.map_or(Valor::int(-1), |(i, _)| m.numero(f64::from(i))))
}

fn ta_find_last_index(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    Ok(percorrer(m, this, a, "findLastIndex", true, verdade)?.map_or(Valor::int(-1), |(i, _)| m.numero(f64::from(i))))
}

fn ta_includes(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "includes")?;
    if n == 0 {
        return Ok(Valor::FALSE);
    }
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    let x = m.arg(a, 0);
    for i in ini..n {
        let v = ler(m, this, i);
        if m.mesmo_valor_zero(v, x) {
            return Ok(Valor::TRUE);
        }
    }
    Ok(Valor::FALSE)
}

fn ta_index_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "indexOf")?;
    if n == 0 {
        return Ok(Valor::int(-1));
    }
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    let x = m.arg(a, 0);
    for i in ini..n {
        if let Some(v) = m.tipado_ler(this, i) {
            if m.identicos(v, x) {
                return Ok(m.numero(f64::from(i)));
            }
        }
    }
    Ok(Valor::int(-1))
}

fn ta_last_index_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "lastIndexOf")?;
    if n == 0 {
        return Ok(Valor::int(-1));
    }
    let de = if a.n > 1 { m.para_inteiro(m.arg(a, 1))? } else { f64::from(n) - 1.0 };
    let k = if de >= 0.0 { de.min(f64::from(n) - 1.0) } else { f64::from(n) + de };
    if k < 0.0 {
        return Ok(Valor::int(-1));
    }
    let x = m.arg(a, 0);
    for i in (0..=k as u32).rev() {
        if let Some(v) = m.tipado_ler(this, i) {
            if m.identicos(v, x) {
                return Ok(m.numero(f64::from(i)));
            }
        }
    }
    Ok(Valor::int(-1))
}

fn ta_join(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "join")?;
    let sv = m.arg(a, 0);
    let sep: Vec<u16> = if sv == Valor::UNDEFINED {
        vec![u16::from(b',')]
    } else {
        let s = m.para_texto(sv)?;
        m.texto_u16_de(s)
    };
    let mut r: Vec<u16> = Vec::new();
    for i in 0..n {
        if i > 0 {
            r.extend_from_slice(&sep);
        }
        if let Some(v) = m.tipado_ler(this, i) {
            let s = m.para_texto(v)?;
            r.extend(m.texto_u16_de(s));
        }
    }
    Ok(m.texto_u16(&r))
}

fn ta_to_locale_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "toLocaleString")?;
    let mut r: Vec<u16> = Vec::new();
    let k = m.chave("toLocaleString");
    for i in 0..n {
        if i > 0 {
            r.push(u16::from(b','));
        }
        let v = ler(m, this, i);
        if !v.e_nulo_ou_indefinido() {
            let f = m.obter(v, ChaveProp::Nome(k))?;
            let x = m.chamar(f, v, &[])?;
            let s = m.para_texto(x)?;
            r.extend(m.texto_u16_de(s));
        }
    }
    Ok(m.texto_u16(&r))
}

fn ta_copy_within(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "copyWithin")?;
    let para = relativo(m, m.arg(a, 0), n, 0)?;
    let de = relativo(m, m.arg(a, 1), n, 0)?;
    let fim = relativo(m, m.arg(a, 2), n, n)?;
    let cont = fim.saturating_sub(de).min(n - para);
    if cont > 0 {
        let Some((t, d, n2)) = vista(m, this) else {
            return m.erro_tipo("Cannot perform copyWithin on a detached or out-of-bounds TypedArray");
        };
        // A cópia para no fim atual (o buffer pode ter encolhido nas conversões).
        let tam = t.tamanho();
        let limite = n2 * tam;
        let (de, para) = (de * tam, para * tam);
        let maior = de.max(para);
        if maior < limite {
            copiar_bytes(m, d + de, d + para, (cont * tam).min(limite - maior));
        }
    }
    Ok(this)
}

fn ta_fill(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = validar(m, this, "fill")?;
    let x = converter(m, t, m.arg(a, 0))?;
    let ini = relativo(m, m.arg(a, 1), n, 0)?;
    let fim = relativo(m, m.arg(a, 2), n, n)?;
    let Some((t, d, n2)) = vista(m, this) else {
        return m.erro_tipo("Cannot perform fill on a detached or out-of-bounds TypedArray");
    };
    for i in ini..fim.min(n2) {
        gravar_cru(m, t, d, i, x);
    }
    Ok(this)
}

fn ta_filter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "filter")?;
    let (f, this_f) = (m.arg(a, 0), m.arg(a, 1));
    funcao(m, f)?;
    let mut ficam = Vec::new();
    for i in 0..n {
        let v = ler(m, this, i);
        let iv = m.numero(f64::from(i));
        let r = m.chamar(f, this_f, &[v, iv, this])?;
        if m.verdadeiro(r) {
            ficam.push(v);
        }
    }
    let nv = m.numero(ficam.len() as f64);
    let o = criar_especie(m, this, &[nv])?;
    for (i, v) in ficam.into_iter().enumerate() {
        gravar_em(m, o, i as u32, v)?;
    }
    Ok(o)
}

fn ta_map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = validar(m, this, "map")?;
    let (f, this_f) = (m.arg(a, 0), m.arg(a, 1));
    funcao(m, f)?;
    let nv = m.numero(f64::from(n));
    let o = criar_especie(m, this, &[nv])?;
    for i in 0..n {
        let v = ler(m, this, i);
        let iv = m.numero(f64::from(i));
        let r = m.chamar(f, this_f, &[v, iv, this])?;
        gravar_em(m, o, i, r)?;
    }
    Ok(o)
}

fn reduzir(m: &mut Motor, this: Valor, a: Args, nome: &str, reverso: bool) -> Res<Valor> {
    let (_, n) = validar(m, this, nome)?;
    let f = m.arg(a, 0);
    funcao(m, f)?;
    let pos = |j: u32| if reverso { n - 1 - j } else { j };
    let mut j = 0;
    let mut acc = if a.n >= 2 {
        m.arg(a, 1)
    } else {
        if n == 0 {
            return m.erro_tipo("Reduce of empty array with no initial value");
        }
        j = 1;
        ler(m, this, pos(0))
    };
    while j < n {
        let i = pos(j);
        let v = ler(m, this, i);
        let iv = m.numero(f64::from(i));
        acc = m.chamar(f, Valor::UNDEFINED, &[acc, v, iv, this])?;
        j += 1;
    }
    Ok(acc)
}

fn ta_reduce(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    reduzir(m, this, a, "reduce", false)
}

fn ta_reduce_right(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    reduzir(m, this, a, "reduceRight", true)
}

fn ta_reverse(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    validar(m, this, "reverse")?;
    let (t, d, n) = vista(m, this).unwrap();
    for i in 0..n / 2 {
        let (x, y) = (ler_cru(m, t, d, i), ler_cru(m, t, d, n - 1 - i));
        gravar_cru(m, t, d, i, y);
        gravar_cru(m, t, d, n - 1 - i, x);
    }
    Ok(this)
}

fn ta_to_reversed(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = validar(m, this, "toReversed")?;
    let o = tipado_vazio(m, t, n)?;
    let (_, de, _) = vista(m, this).unwrap();
    let (_, para, _) = vista(m, o).unwrap();
    for i in 0..n {
        let x = ler_cru(m, t, de, n - 1 - i);
        gravar_cru(m, t, para, i, x);
    }
    Ok(o)
}

/// `alvo.set(origem, deslocamento)`.
fn ta_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = exigir_tipado(m, this, "set")?;
    let x = m.arg(a, 0);
    let desl = m.para_inteiro(m.arg(a, 1))?;
    if desl < 0.0 {
        return m.erro_alcance("offset is out of bounds");
    }
    let Some((_, _, n)) = vista(m, this) else {
        return m.erro_tipo("Cannot perform set on a detached or out-of-bounds TypedArray");
    };
    if classe(m, x) == Some(Classe::ArrayTipado) {
        let Some((ts, _, k)) = vista(m, x) else {
            return m.erro_tipo("Source TypedArray is detached or out of bounds");
        };
        if desl + f64::from(k) > f64::from(n) {
            return m.erro_alcance("offset is out of bounds");
        }
        if ts.e_bigint() != t.e_bigint() {
            return m.erro_tipo("Cannot mix BigInt and other types, use explicit conversions");
        }
        let desl = desl as u32;
        let (_, de, _) = vista(m, x).unwrap();
        let (_, para, _) = vista(m, this).unwrap();
        if ts == t {
            // Cópia de bytes: certa também quando as duas vistas dividem o buffer.
            copiar_bytes(m, de, para + desl * t.tamanho(), k * t.tamanho());
        } else {
            // Lê tudo antes de gravar (o buffer pode ser o mesmo).
            let itens: Vec<Bruto> = (0..k).map(|i| ler_cru(m, ts, de, i)).collect();
            for (i, v) in itens.into_iter().enumerate() {
                gravar_cru(m, t, para, desl + i as u32, v);
            }
        }
        return Ok(Valor::UNDEFINED);
    }
    let o = m.para_objeto(x)?;
    let k = m.comprimento_de(o)?;
    if desl + k as f64 > f64::from(n) {
        return m.erro_alcance("offset is out of bounds");
    }
    let desl = desl as u32;
    for i in 0..k as u32 {
        let v = m.obter_indice(o, u64::from(i))?;
        m.tipado_gravar(this, desl + i, v)?;
    }
    Ok(Valor::UNDEFINED)
}

/// Cópia de `ini..fim` num array da espécie.
fn ta_slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = validar(m, this, "slice")?;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fim = relativo(m, m.arg(a, 1), n, n)?;
    let nv = m.numero(f64::from(fim.saturating_sub(ini)));
    let o = criar_especie(m, this, &[nv])?;
    if fim > ini {
        let Some((_, de, n2)) = vista(m, this) else {
            return m.erro_tipo("Cannot perform slice on a detached or out-of-bounds TypedArray");
        };
        let fim = fim.min(n2);
        let to = tipo_de(m, o);
        if to == t {
            // Byte a byte para a frente, como a norma: importa quando a espécie devolve uma
            // vista sobre o mesmo buffer.
            let (_, para, _) = vista(m, o).unwrap();
            let tam = t.tamanho();
            let de = de + ini * tam;
            for j in 0..fim.saturating_sub(ini) * tam {
                let b = m.g.bytes(de + j, 1)[0];
                m.g.bytes_mut(para + j, 1)[0] = b;
            }
        } else {
            for (j, i) in (ini..fim).enumerate() {
                let v = ler(m, this, i);
                gravar_em(m, o, j as u32, v)?;
            }
        }
    }
    Ok(o)
}

/// Nova vista da espécie sobre o mesmo buffer.
fn ta_subarray(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = exigir_tipado(m, this, "subarray")?;
    let buf = Valor(m.interno(this, 0));
    let n = m.tipado_len(this);
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fv = m.arg(a, 1);
    let fim = relativo(m, fv, n, n)?;
    let desl = m.numero(f64::from(m.interno(this, 2)) + f64::from(ini) * f64::from(t.tamanho()));
    if m.interno(this, 3) == AUTO && fv == Valor::UNDEFINED {
        return criar_especie(m, this, &[buf, desl]);
    }
    let nv = m.numero(f64::from(fim.saturating_sub(ini)));
    criar_especie(m, this, &[buf, desl, nv])
}

/// Itens em ordem (numérica, ou pelo comparador), lidos uma vez só.
fn ordenados(m: &mut Motor, this: Valor, t: Tipo, n: u32, f: Valor) -> Res<Vec<Bruto>> {
    let (_, d, _) = vista(m, this).unwrap();
    let mut v: Vec<Bruto> = (0..n).map(|i| ler_cru(m, t, d, i)).collect();
    if f == Valor::UNDEFINED {
        v.sort_by(|a, b| match (*a, *b) {
            (Bruto::B(x), Bruto::B(y)) if t == Tipo::I64 => (x as i64).cmp(&(y as i64)),
            (Bruto::B(x), Bruto::B(y)) => x.cmp(&y),
            (Bruto::N(a), Bruto::N(b)) => match (a.is_nan(), b.is_nan()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                // -0 antes de +0.
                _ => a.partial_cmp(&b).unwrap().then_with(|| b.is_sign_positive().cmp(&a.is_sign_positive()).reverse()),
            },
            _ => Ordering::Equal,
        });
        return Ok(v);
    }
    let mut vs: Vec<Valor> = v.into_iter().map(|x| valor_de(m, t, x)).collect();
    super::array::ordenar(m, &mut vs, f)?;
    let mut r = Vec::with_capacity(vs.len());
    for x in vs {
        r.push(converter(m, t, x)?);
    }
    Ok(r)
}

fn comparador(m: &mut Motor, f: Valor) -> Res<()> {
    if f != Valor::UNDEFINED && !m.e_funcao(f) {
        return m.erro_tipo("The comparison function must be either a function or undefined");
    }
    Ok(())
}

fn ta_sort(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    comparador(m, f)?;
    let (t, n) = validar(m, this, "sort")?;
    let v = ordenados(m, this, t, n, f)?;
    // O comparador pode ter encolhido o buffer: grava só o que ainda cabe.
    if let Some((t, d, n2)) = vista(m, this) {
        for (i, x) in v.into_iter().enumerate().take(n2 as usize) {
            gravar_cru(m, t, d, i as u32, x);
        }
    }
    Ok(this)
}

fn ta_to_sorted(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let f = m.arg(a, 0);
    comparador(m, f)?;
    let (t, n) = validar(m, this, "toSorted")?;
    let o = tipado_vazio(m, t, n)?;
    let v = ordenados(m, this, t, n, f)?;
    let (_, d, _) = vista(m, o).unwrap();
    for (i, x) in v.into_iter().enumerate() {
        gravar_cru(m, t, d, i as u32, x);
    }
    Ok(o)
}

fn ta_with(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = validar(m, this, "with")?;
    let rel = m.para_inteiro(m.arg(a, 0))?;
    let i = if rel < 0.0 { f64::from(n) + rel } else { rel };
    let x = converter(m, t, m.arg(a, 1))?;
    if !(0.0..f64::from(m.tipado_len(this))).contains(&i) {
        return m.erro_alcance("Invalid typed array index");
    }
    let i = i as u32;
    let o = tipado_vazio(m, t, n)?;
    for k in 0..n {
        let (_, d, _) = vista(m, o).unwrap();
        if k == i {
            gravar_cru(m, t, d, k, x);
        } else {
            let v = ler(m, this, k);
            gravar_em(m, o, k, v)?;
        }
    }
    Ok(o)
}

// ---------------------------------------------------------------- DataView

fn dv_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Constructor DataView requires 'new'");
    }
    let buf = m.arg(a, 0);
    if !e_buffer(m, buf) {
        return m.erro_tipo("First argument to DataView constructor must be an ArrayBuffer");
    }
    let desl = para_indice(m, m.arg(a, 1), "DataView offset")?;
    if desligado(m, buf) {
        return m.erro_tipo("Cannot construct a DataView on a detached ArrayBuffer");
    }
    let total = buf_len(m, buf);
    if desl > total {
        return m.erro_alcance(&format!("Start offset {desl} is outside the bounds of the buffer"));
    }
    let lv = m.arg(a, 2);
    let n = if lv == Valor::UNDEFINED {
        if redimensionavel(m, buf) {
            AUTO
        } else {
            total - desl
        }
    } else {
        let n = para_indice(m, lv, "DataView length")?;
        if u64::from(desl) + u64::from(n) > u64::from(total) {
            return m.erro_alcance(&format!("Invalid DataView length {n}"));
        }
        n
    };
    let padrao = m.obter(m.chamado, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = proto_de_nt(m, nt, padrao)?;
    // Pegar o `prototype` pode ter desligado ou encolhido o buffer.
    if desligado(m, buf) {
        return m.erro_tipo("Cannot construct a DataView on a detached ArrayBuffer");
    }
    let total = buf_len(m, buf);
    if desl > total || (lv != Valor::UNDEFINED && u64::from(desl) + u64::from(n) > u64::from(total)) {
        return m.erro_alcance("DataView is outside the bounds of the buffer");
    }
    let f = m.forma_para(Classe::DataView, p, 0);
    let o = m.novo_objeto(f);
    m.por_interno_valor(o, 0, buf);
    m.por_interno(o, 2, desl);
    m.por_interno(o, 3, n);
    Ok(o)
}

fn exigir_dv(m: &mut Motor, this: Valor) -> Res<()> {
    if classe(m, this) == Some(Classe::DataView) {
        return Ok(());
    }
    m.erro_tipo("this is not a DataView")
}

/// Começo e tamanho atual da vista; TypeError se desligada ou fora do buffer.
fn vista_dv(m: &mut Motor, this: Valor) -> Res<(u32, u32)> {
    match medir(m, this, 1) {
        Some(v) => Ok(v),
        None => m.erro_tipo("DataView is detached or out of bounds"),
    }
}

fn dv_buffer(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    exigir_dv(m, this)?;
    Ok(Valor(m.interno(this, 0)))
}

fn dv_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    exigir_dv(m, this)?;
    let (_, n) = vista_dv(m, this)?;
    Ok(m.numero(f64::from(n)))
}

fn dv_byte_offset(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    exigir_dv(m, this)?;
    vista_dv(m, this)?;
    Ok(m.numero(f64::from(m.interno(this, 2))))
}

/// Onde mora o item `t` no byte `i` da vista (ou TypeError/RangeError).
fn posicao(m: &mut Motor, this: Valor, i: u32, t: Tipo) -> Res<u32> {
    let (d, n) = vista_dv(m, this)?;
    if u64::from(i) + u64::from(t.tamanho()) > u64::from(n) {
        return m.erro_alcance("Offset is outside the bounds of the DataView");
    }
    Ok(d + i)
}

fn dv_get(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = Tipo::de(m.capturado().como_int() as u32);
    exigir_dv(m, this)?;
    let i = para_indice(m, m.arg(a, 0), "DataView index")?;
    let pequeno = m.verdadeiro(m.arg(a, 1));
    let d = posicao(m, this, i, t)?;
    let x = t.ler(m.g.bytes(d, t.tamanho()), !pequeno);
    Ok(valor_de(m, t, x))
}

fn dv_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = Tipo::de(m.capturado().como_int() as u32);
    exigir_dv(m, this)?;
    let i = para_indice(m, m.arg(a, 0), "DataView index")?;
    let x = converter(m, t, m.arg(a, 1))?;
    let pequeno = m.verdadeiro(m.arg(a, 2));
    let d = posicao(m, this, i, t)?;
    t.gravar(m.g.bytes_mut(d, t.tamanho()), x, !pequeno);
    Ok(Valor::UNDEFINED)
}

// ---------------------------------------------------------------- Atomics

/// ValidateIntegerTypedArray + ValidateAtomicAccess: tipo e índice do item.
fn atomico(m: &mut Motor, ta: Valor, idx: Valor, esperavel: bool) -> Res<(Tipo, u32)> {
    let (t, n) = tipo_atomico(m, ta, esperavel)?;
    indice_atomico(m, t, n, idx)
}

/// ValidateIntegerTypedArray: tipo e comprimento.
fn tipo_atomico(m: &mut Motor, ta: Valor, esperavel: bool) -> Res<(Tipo, u32)> {
    let (t, n) = validar(m, ta, "Atomics operation")?;
    let ok = if esperavel { matches!(t, Tipo::I32 | Tipo::I64) } else { t.atomico() };
    if !ok {
        return m.erro_tipo(&format!("Atomics operations are not allowed on {}", t.nome()));
    }
    Ok((t, n))
}

/// ValidateAtomicAccess.
fn indice_atomico(m: &mut Motor, t: Tipo, n: u32, idx: Valor) -> Res<(Tipo, u32)> {
    let i = para_indice(m, idx, "atomic access index")?;
    if i >= n {
        return m.erro_alcance("Invalid atomic access index");
    }
    Ok((t, i))
}

/// RevalidateAtomicAccess: endereço do item depois das conversões.
fn reconferir(m: &mut Motor, ta: Valor, i: u32) -> Res<(Tipo, u32)> {
    match vista(m, ta) {
        None => m.erro_tipo("TypedArray is detached or out of bounds"),
        Some((_, _, n)) if i >= n => m.erro_alcance("Invalid atomic access index"),
        Some((t, d, _)) => Ok((t, d + i * t.tamanho())),
    }
}

/// O valor do operando: ToBigInt, ou ToIntegerOrInfinity (com -0 virando +0).
fn operando(m: &mut Motor, t: Tipo, v: Valor) -> Res<(Valor, u64)> {
    if t.e_bigint() {
        let g = m.para_bigint(v)?;
        let u = bits64(&g);
        return Ok((m.novo_bigint(&g), u));
    }
    let x = m.para_inteiro(v)? + 0.0;
    Ok((m.numero(x), u64::from(int32_cru(x))))
}

fn ler_bits(m: &Motor, d: u32, tam: u32) -> u64 {
    m.g.bytes(d, tam).iter().rev().fold(0, |acc, &b| acc << 8 | u64::from(b))
}

fn gravar_bits(m: &mut Motor, d: u32, tam: u32, u: u64) {
    m.g.bytes_mut(d, tam).copy_from_slice(&u.to_le_bytes()[..tam as usize]);
}

/// AtomicReadModifyWrite: devolve o valor velho.
fn modificar(m: &mut Motor, a: Args, op: fn(u64, u64) -> u64) -> Res<Valor> {
    let ta = m.arg(a, 0);
    let (t, i) = atomico(m, ta, m.arg(a, 1), false)?;
    let (_, v) = operando(m, t, m.arg(a, 2))?;
    let (t, d) = reconferir(m, ta, i)?;
    let velho = t.ler(m.g.bytes(d, t.tamanho()), false);
    let tam = t.tamanho();
    let novo = op(ler_bits(m, d, tam), v);
    gravar_bits(m, d, tam, novo);
    Ok(valor_de(m, t, velho))
}

fn at_add(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    modificar(m, a, u64::wrapping_add)
}

fn at_sub(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    modificar(m, a, u64::wrapping_sub)
}

fn at_and(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    modificar(m, a, |x, y| x & y)
}

fn at_or(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    modificar(m, a, |x, y| x | y)
}

fn at_xor(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    modificar(m, a, |x, y| x ^ y)
}

fn at_exchange(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    modificar(m, a, |_, y| y)
}

fn at_compare_exchange(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let ta = m.arg(a, 0);
    let (t, i) = atomico(m, ta, m.arg(a, 1), false)?;
    let (_, esperado) = operando(m, t, m.arg(a, 2))?;
    let (_, novo) = operando(m, t, m.arg(a, 3))?;
    let (t, d) = reconferir(m, ta, i)?;
    let tam = t.tamanho();
    let mascara = if tam == 8 { u64::MAX } else { (1u64 << (tam * 8)) - 1 };
    let velho = t.ler(m.g.bytes(d, tam), false);
    if ler_bits(m, d, tam) == esperado & mascara {
        gravar_bits(m, d, tam, novo);
    }
    Ok(valor_de(m, t, velho))
}

fn at_load(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let ta = m.arg(a, 0);
    let (_, i) = atomico(m, ta, m.arg(a, 1), false)?;
    let (t, d) = reconferir(m, ta, i)?;
    let x = t.ler(m.g.bytes(d, t.tamanho()), false);
    Ok(valor_de(m, t, x))
}

fn at_store(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let ta = m.arg(a, 0);
    let (t, i) = atomico(m, ta, m.arg(a, 1), false)?;
    let (v, u) = operando(m, t, m.arg(a, 2))?;
    let (t, d) = reconferir(m, ta, i)?;
    gravar_bits(m, d, t.tamanho(), u);
    Ok(v)
}

fn at_is_lock_free(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let n = m.para_inteiro(m.arg(a, 0))?;
    Ok(Valor::bool(n == 1.0 || n == 2.0 || n == 4.0 || n == 8.0))
}

fn at_pause(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let n = m.arg(a, 0);
    if n != Valor::UNDEFINED {
        let x = if m.e_numero(n) { m.f64_de(n) } else { f64::NAN };
        if !x.is_finite() || x.trunc() != x {
            return m.erro_tipo("Atomics.pause: argument must be an integral Number");
        }
    }
    Ok(Valor::UNDEFINED)
}

/// DoWait até a comparação: (valor atual igual ao esperado, tempo em ms, buffer, byte).
fn esperar(m: &mut Motor, a: Args) -> Res<(bool, f64, Valor, u32)> {
    let ta = m.arg(a, 0);
    let (t, n) = tipo_atomico(m, ta, true)?;
    let buf = Valor(m.interno(ta, 0));
    if !compartilhado(m, buf) {
        return m.erro_tipo("Atomics.wait requires a shared typed array");
    }
    let (t, i) = indice_atomico(m, t, n, m.arg(a, 1))?;
    let v = if t == Tipo::I64 {
        let g = m.para_bigint(m.arg(a, 2))?;
        bits64(&g)
    } else {
        u64::from(m.para_int32(m.arg(a, 2))? as u32)
    };
    let q = m.para_numero(m.arg(a, 3))?;
    let tempo = if q.is_nan() { f64::INFINITY } else { q.max(0.0) };
    let (t, d) = reconferir(m, ta, i)?;
    let tam = t.tamanho();
    let mascara = if tam == 8 { u64::MAX } else { u64::from(u32::MAX) };
    let byte = m.interno(ta, 2) + i * tam;
    Ok((ler_bits(m, d, tam) == v & mascara, tempo, buf, byte))
}

fn at_wait(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    esperar(m, a)?;
    m.erro_tipo("Atomics.wait cannot be called in this context")
}

fn at_wait_async(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (igual, tempo, buf, byte) = esperar(m, a)?;
    let o = m.objeto_vazio();
    let ka = m.chave("async");
    let (assinc, valor) = if !igual {
        (false, m.texto("not-equal"))
    } else if tempo == 0.0 {
        (false, m.texto("timed-out"))
    } else {
        let p = m.nova_promessa();
        let l = m.intr.atomics_esperas;
        let mut v = Vec::new();
        if l != Valor::UNDEFINED {
            for j in 0..m.comprimento_de(l)? {
                v.push(m.obter_indice(l, j)?);
            }
        }
        v.extend([buf, Valor::int(byte as i32), p]);
        m.intr.atomics_esperas = m.array_de(&v);
        (true, p)
    };
    m.definir_propria(o, ka, Valor::bool(assinc), attr::PADRAO);
    m.definir_propria(o, fixos::VALUE, valor, attr::PADRAO);
    Ok(o)
}

fn at_notify(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let ta = m.arg(a, 0);
    let (t, i) = atomico(m, ta, m.arg(a, 1), true)?;
    let c = m.arg(a, 2);
    let mut cont = if c == Valor::UNDEFINED { f64::INFINITY } else { m.para_inteiro(c)?.max(0.0) };
    let buf = Valor(m.interno(ta, 0));
    let l = m.intr.atomics_esperas;
    if !compartilhado(m, buf) || l == Valor::UNDEFINED {
        return Ok(Valor::int(0));
    }
    // Acorda, na ordem em que esperaram, quem espera neste byte do mesmo buffer.
    let byte = Valor::int((m.interno(ta, 2) + i * t.tamanho()) as i32);
    let n = m.comprimento_de(l)? as u32;
    let (mut ficam, mut acordados) = (Vec::new(), 0);
    for j in (0..n).step_by(3) {
        let (b, x, p) = (m.obter_indice(l, u64::from(j))?, m.obter_indice(l, u64::from(j + 1))?, m.obter_indice(l, u64::from(j + 2))?);
        if cont > 0.0 && b == buf && x == byte {
            cont -= 1.0;
            acordados += 1;
            let ok = m.texto("ok");
            m.resolver_promessa(p, ok);
        } else {
            ficam.extend([b, x, p]);
        }
    }
    m.intr.atomics_esperas = m.array_de(&ficam);
    Ok(Valor::int(acordados))
}

fn f16round(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let x = m.para_numero(m.arg(a, 0))?;
    Ok(m.numero(f16_para_f64(f64_para_f16(x))))
}

#[cfg(test)]
mod testes {
    use super::{f16_para_f64, f64_para_f16, Bruto, Tipo};

    #[test]
    fn conversoes_dos_itens() {
        let mut b = [0u8; 8];
        Tipo::U8c.gravar(&mut b, Bruto::N(2.5), false);
        assert_eq!(b[0], 2);
        Tipo::U8c.gravar(&mut b, Bruto::N(3.5), false);
        assert_eq!(b[0], 4);
        Tipo::I8.gravar(&mut b, Bruto::N(200.0), false);
        assert_eq!(Tipo::I8.ler(&b, false), Bruto::N(-56.0));
        Tipo::U32.gravar(&mut b, Bruto::N(-1.0), false);
        assert_eq!(Tipo::U32.ler(&b, false), Bruto::N(4_294_967_295.0));
        Tipo::I16.gravar(&mut b, Bruto::N(258.0), true);
        assert_eq!(&b[..2], &[1, 2]);
        Tipo::F64.gravar(&mut b, Bruto::N(0.1), false);
        assert_eq!(Tipo::F64.ler(&b, false), Bruto::N(0.1));
        Tipo::I64.gravar(&mut b, Bruto::B(u64::MAX), false);
        assert_eq!(Tipo::U64.ler(&b, false), Bruto::B(u64::MAX));
    }

    #[test]
    fn float16() {
        let ida = |x: f64| f16_para_f64(f64_para_f16(x));
        assert_eq!(ida(1.337), 1.3369140625);
        assert_eq!(ida(65504.0), 65504.0);
        assert_eq!(ida(65519.99), 65504.0);
        assert_eq!(ida(65520.0), f64::INFINITY);
        assert_eq!(ida(5.960464477539063e-8), 5.960464477539063e-8);
        assert_eq!(ida(2.980232238769531e-8), 0.0);
        assert_eq!(ida(2.9802322387695312e-8 * 1.0000001), 5.960464477539063e-8);
        assert!(ida(-0.0).is_sign_negative());
        assert!(ida(f64::NAN).is_nan());
        // Empate entre 1 e 1+2^-10 vai para o par (1); arredondar via f32 erraria aqui.
        assert_eq!(ida(1.0 + 2f64.powi(-11)), 1.0);
        assert_eq!(ida(1.0 + 2f64.powi(-11) + 2f64.powi(-40)), 1.0 + 2f64.powi(-10));
    }
}
