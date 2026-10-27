//! ArrayBuffer, os arrays tipados (`Uint8Array`…) e DataView.
//!
//! Os bytes moram na gaiola, num bloco `BYTES` ([forma][tamanho][bytes…]) guardado no campo
//! interno 0 do ArrayBuffer. Array tipado e DataView guardam [buffer][tipo][deslocamento em
//! bytes][comprimento] (comprimento em itens no array tipado, em bytes no DataView).
//!
//! Os métodos que a norma define iguais aos do Array (join, indexOf, forEach…) são os próprios
//! métodos genéricos do `Array.prototype`; aqui ficam só os que mudam (map, filter, slice, sort,
//! set, subarray…).
//!
//! shortcut: sem BigInt64Array/BigUint64Array, sem buffer redimensionável,
//! `transfer` nem SharedArrayBuffer; buffer desligado nunca acontece. Upgrade: quando um site pedir.

use std::cmp::Ordering;

use crate::formas::{self, attr, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

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
}

const TIPOS: [(Tipo, &str); 9] = [
    (Tipo::I8, "Int8Array"),
    (Tipo::U8, "Uint8Array"),
    (Tipo::U8c, "Uint8ClampedArray"),
    (Tipo::I16, "Int16Array"),
    (Tipo::U16, "Uint16Array"),
    (Tipo::I32, "Int32Array"),
    (Tipo::U32, "Uint32Array"),
    (Tipo::F32, "Float32Array"),
    (Tipo::F64, "Float64Array"),
];

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
            Tipo::I16 | Tipo::U16 => 2,
            Tipo::I32 | Tipo::U32 | Tipo::F32 => 4,
            Tipo::F64 => 8,
        }
    }

    fn nome(self) -> &'static str {
        TIPOS[self as usize].1
    }

    /// Lê o item de `b` (little-endian salvo `grande`).
    fn ler(self, b: &[u8], grande: bool) -> f64 {
        macro_rules! de {
            ($t:ty, $n:expr) => {{
                let mut x = [0u8; $n];
                x.copy_from_slice(&b[..$n]);
                if grande {
                    <$t>::from_be_bytes(x)
                } else {
                    <$t>::from_le_bytes(x)
                }
            }};
        }
        match self {
            Tipo::I8 => f64::from(b[0] as i8),
            Tipo::U8 | Tipo::U8c => f64::from(b[0]),
            Tipo::I16 => f64::from(de!(i16, 2)),
            Tipo::U16 => f64::from(de!(u16, 2)),
            Tipo::I32 => f64::from(de!(i32, 4)),
            Tipo::U32 => f64::from(de!(u32, 4)),
            Tipo::F32 => f64::from(de!(f32, 4)),
            Tipo::F64 => de!(f64, 8),
        }
    }

    /// Grava `x` (já número) convertido como a norma manda (ToInt8, ToUint8Clamp…).
    fn gravar(self, b: &mut [u8], x: f64, grande: bool) {
        macro_rules! em {
            ($v:expr) => {{
                let v = $v;
                let bytes = if grande { v.to_be_bytes() } else { v.to_le_bytes() };
                b[..bytes.len()].copy_from_slice(&bytes);
            }};
        }
        let int = || -> u32 {
            if x.is_finite() {
                x.trunc().rem_euclid(4_294_967_296.0) as u32
            } else {
                0
            }
        };
        match self {
            Tipo::I8 | Tipo::U8 => b[0] = int() as u8,
            Tipo::U8c => {
                b[0] = if x.is_nan() || x <= 0.0 {
                    0
                } else if x >= 255.0 {
                    255
                } else {
                    // Arredonda para o par mais próximo.
                    let f = x.floor();
                    let r = x - f;
                    (if r > 0.5 || (r == 0.5 && f % 2.0 != 0.0) { f + 1.0 } else { f }) as u8
                }
            }
            Tipo::I16 | Tipo::U16 => em!(int() as u16),
            Tipo::I32 | Tipo::U32 => em!(int()),
            Tipo::F32 => em!(x as f32),
            Tipo::F64 => em!(x),
        }
    }
}

// ---------------------------------------------------------------- acesso cru

/// Bloco `BYTES` novo, zerado.
fn novos_bytes(m: &mut Motor, n: u32) -> Valor {
    let d = m.g.alocar(8 + n);
    m.g.por_u32(d, formas::BYTES.0);
    m.g.por_u32(d + 4, n);
    m.g.bytes_mut(d + 8, n).fill(0);
    Valor::de_deslocamento(d)
}

/// Deslocamento na gaiola do primeiro byte do ArrayBuffer e o tamanho dele.
fn bytes_do_buffer(m: &Motor, buf: Valor) -> (u32, u32) {
    let b = Valor(m.interno(buf, 0));
    let d = b.deslocamento();
    (d + 8, m.g.u32_em(d + 4))
}

fn classe(m: &Motor, v: Valor) -> Option<Classe> {
    m.e_objeto(v).then(|| m.formas.get(m.forma(v)).classe)
}

/// (tipo, onde começa o item 0 na gaiola, comprimento) de um array tipado.
#[inline]
fn partes(m: &Motor, o: Valor) -> (Tipo, u32, u32) {
    let buf = Valor(m.interno(o, 0));
    let (d, _) = bytes_do_buffer(m, buf);
    (Tipo::de(m.interno(o, 1)), d + m.interno(o, 2), m.interno(o, 3))
}

impl Motor {
    /// Item `i` de um array tipado (None = fora do comprimento).
    pub(crate) fn tipado_ler(&mut self, o: Valor, i: u32) -> Option<Valor> {
        let (t, d, n) = partes(self, o);
        if i >= n {
            return None;
        }
        let tam = t.tamanho();
        let x = t.ler(self.g.bytes(d + i * tam, tam), false);
        Some(match t {
            Tipo::I8 | Tipo::U8 | Tipo::U8c | Tipo::I16 | Tipo::U16 => Valor::int(x as i32),
            _ => self.numero(x),
        })
    }

    /// `o[i] = v` num array tipado (fora do comprimento não faz nada, como manda a norma).
    pub(crate) fn tipado_gravar(&mut self, o: Valor, i: u32, v: Valor) -> Res<()> {
        let x = if v.e_int() { f64::from(v.como_int()) } else { self.para_numero(v)? };
        let (t, d, n) = partes(self, o);
        if i < n {
            let tam = t.tamanho();
            t.gravar(self.g.bytes_mut(d + i * tam, tam), x, false);
        }
        Ok(())
    }

    pub(crate) fn tipado_len(&self, o: Valor) -> u32 {
        self.interno(o, 3)
    }
}

// ---------------------------------------------------------------- instalar

pub(super) fn instalar(m: &mut Motor) {
    let op = m.intr.objeto_proto;
    let g = m.intr.global;

    // ---- ArrayBuffer
    let abp = proto_com(m, op);
    let ab = m.nativa("ArrayBuffer", 1, ab_construtor, true);
    ligar(m, ab, "ArrayBuffer", abp, true);
    m.metodo(ab, "isView", 1, ab_is_view);
    getter(m, abp, "byteLength", ab_byte_length);
    getter(m, abp, "maxByteLength", ab_byte_length);
    getter(m, abp, "resizable", falso);
    getter(m, abp, "detached", falso);
    m.metodo(abp, "slice", 2, ab_slice);
    etiqueta(m, abp, "ArrayBuffer");
    m.intr.array_buffer_proto = abp;

    // ---- %TypedArray%
    let tap = proto_com(m, op);
    let ta = m.nativa("TypedArray", 0, ta_abstrato, true);
    m.definir_propria(ta, fixos::PROTOTYPE, tap, 0);
    m.definir_propria(tap, fixos::CONSTRUCTOR, ta, attr::METODO);
    m.metodo(ta, "from", 1, ta_from);
    m.metodo(ta, "of", 0, ta_of);
    getter(m, tap, "buffer", ta_buffer);
    getter(m, tap, "byteLength", ta_byte_length);
    getter(m, tap, "byteOffset", ta_byte_offset);
    getter(m, tap, "length", ta_length);
    let tag = m.nativa("get [Symbol.toStringTag]", 0, ta_tag, false);
    m.definir_acessor(tap, ChaveProp::Nome(simb::TO_STRING_TAG), tag, false, false);
    let proprios: &[(&str, u32, FnNativa)] = &[
        ("set", 1, ta_set),
        ("subarray", 2, ta_subarray),
        ("slice", 2, ta_slice),
        ("map", 1, ta_map),
        ("filter", 1, ta_filter),
        ("sort", 1, ta_sort),
        ("toSorted", 1, ta_to_sorted),
        ("toReversed", 0, ta_to_reversed),
        ("with", 2, ta_with),
    ];
    for &(nome, n, f) in proprios {
        m.metodo(tap, nome, n, f);
    }
    // Genéricos do Array.prototype que a norma repete igual para os arrays tipados.
    let ap = m.intr.array_proto;
    for nome in [
        "at", "copyWithin", "entries", "every", "fill", "find", "findIndex", "findLast", "findLastIndex", "forEach", "includes", "indexOf", "join", "keys", "lastIndexOf", "reduce",
        "reduceRight", "reverse", "some", "toLocaleString", "toString", "values",
    ] {
        let c = m.chave(nome);
        let f = m.obter(ap, ChaveProp::Nome(c)).unwrap_or(Valor::UNDEFINED);
        m.definir_propria(tap, c, f, attr::METODO);
        if nome == "values" {
            m.definir_propria(tap, simb::ITERATOR, f, attr::METODO);
        }
    }

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
        ligar(m, c, nome, p, true);
        m.intr.tipados_protos[t as usize] = p;
    }

    // ---- DataView
    let dvp = proto_com(m, op);
    let dv = m.nativa("DataView", 1, dv_construtor, true);
    ligar(m, dv, "DataView", dvp, true);
    getter(m, dvp, "buffer", ta_buffer);
    getter(m, dvp, "byteLength", dv_byte_length);
    getter(m, dvp, "byteOffset", ta_byte_offset);
    let ler = m.registrar_nativa("get", dv_get, false);
    let gravar = m.registrar_nativa("set", dv_set, false);
    for (t, nome) in TIPOS {
        if t == Tipo::U8c {
            continue;
        }
        let base = &nome[..nome.len() - "Array".len()];
        for (pre, id, n) in [("get", ler, 1), ("set", gravar, 2)] {
            let f = m.nativa_com(id, n, Valor::int(t.numero() as i32));
            let c = m.chave(&format!("{pre}{base}"));
            m.definir_propria(dvp, c, f, attr::METODO);
        }
    }
    etiqueta(m, dvp, "DataView");
    let _ = g;
}

fn proto_com(m: &mut Motor, pai: Valor) -> Valor {
    let f = m.forma_para(Classe::Comum, pai, 0);
    m.novo_objeto(f)
}

fn ligar(m: &mut Motor, c: Valor, nome: &str, proto: Valor, global: bool) {
    m.definir_propria(c, fixos::PROTOTYPE, proto, 0);
    m.definir_propria(proto, fixos::CONSTRUCTOR, c, attr::METODO);
    if global {
        let k = m.chave(nome);
        let g = m.intr.global;
        m.definir_propria(g, k, c, attr::METODO);
    }
}

fn getter(m: &mut Motor, obj: Valor, nome: &'static str, f: FnNativa) {
    let g = m.nativa(nome, 0, f, false);
    let c = m.chave(nome);
    m.definir_acessor(obj, ChaveProp::Nome(c), g, false, false);
}

fn etiqueta(m: &mut Motor, obj: Valor, nome: &str) {
    let t = m.texto(nome);
    m.definir_propria(obj, simb::TO_STRING_TAG, t, attr::CONFIGURAVEL);
}

fn falso(_: &mut Motor, _: Valor, _: Args, _: Valor) -> Res<Valor> {
    Ok(Valor::FALSE)
}

/// Protótipo de `new X()`: o `prototype` de `nt`, ou `padrao`.
fn proto_de_nt(m: &mut Motor, nt: Valor, padrao: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return Ok(padrao);
    }
    let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
    Ok(if m.e_objeto(p) { p } else { padrao })
}

/// ToIndex (§7.1.22).
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

// ---------------------------------------------------------------- ArrayBuffer

fn novo_buffer(m: &mut Motor, n: u32, proto: Valor) -> Valor {
    let f = m.forma_para(Classe::ArrayBuffer, proto, 0);
    let o = m.novo_objeto(f);
    let b = novos_bytes(m, n);
    m.por_interno_valor(o, 0, b);
    o
}

fn ab_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Constructor ArrayBuffer requires 'new'");
    }
    let n = para_indice(m, m.arg(a, 0), "array buffer length")?;
    let padrao = m.intr.array_buffer_proto;
    let p = proto_de_nt(m, nt, padrao)?;
    Ok(novo_buffer(m, n, p))
}

fn buffer_de(m: &mut Motor, this: Valor, nome: &str) -> Res<Valor> {
    if classe(m, this) == Some(Classe::ArrayBuffer) {
        return Ok(this);
    }
    m.erro_tipo(&format!("Method ArrayBuffer.prototype.{nome} called on incompatible receiver"))
}

fn ab_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "byteLength")?;
    Ok(m.numero(f64::from(bytes_do_buffer(m, b).1)))
}

fn ab_is_view(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    Ok(Valor::bool(matches!(classe(m, v), Some(Classe::ArrayTipado | Classe::DataView))))
}

fn ab_slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let b = buffer_de(m, this, "slice")?;
    let n = bytes_do_buffer(m, b).1;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fim = relativo(m, m.arg(a, 1), n, n)?.max(ini);
    let p = m.intr.array_buffer_proto;
    let novo = novo_buffer(m, fim - ini, p);
    let (de, _) = bytes_do_buffer(m, b);
    let (para, _) = bytes_do_buffer(m, novo);
    copiar_bytes(m, de + ini, para, fim - ini);
    Ok(novo)
}

fn copiar_bytes(m: &mut Motor, de: u32, para: u32, n: u32) {
    let v = m.g.bytes(de, n).to_vec();
    m.g.bytes_mut(para, n).copy_from_slice(&v);
}

// ---------------------------------------------------------------- arrays tipados

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

/// Array tipado zerado com `n` itens e o protótipo padrão do tipo.
fn tipado_vazio(m: &mut Motor, t: Tipo, n: u32) -> Res<Valor> {
    let bytes = n.checked_mul(t.tamanho()).filter(|&b| b < u32::MAX - 16);
    let Some(bytes) = bytes else { return m.erro_alcance("Invalid typed array length") };
    let abp = m.intr.array_buffer_proto;
    let buf = novo_buffer(m, bytes, abp);
    let p = m.intr.tipados_protos[t as usize];
    Ok(novo_tipado(m, t, buf, 0, n, p))
}

fn ta_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let t = Tipo::de(m.capturado().como_int() as u32);
    if nt == Valor::UNDEFINED {
        return m.erro_tipo(&format!("Constructor {} requires 'new'", t.nome()));
    }
    let padrao = m.intr.tipados_protos[t as usize];
    let p = proto_de_nt(m, nt, padrao)?;
    let x = m.arg(a, 0);
    let o = match classe(m, x) {
        None => {
            let n = para_indice(m, x, "typed array length")?;
            tipado_vazio(m, t, n)?
        }
        Some(Classe::ArrayBuffer) => {
            let tam = t.tamanho();
            let desl = para_indice(m, m.arg(a, 1), "offset")?;
            if desl % tam != 0 {
                return m.erro_alcance(&format!("start offset of {} should be a multiple of {tam}", t.nome()));
            }
            let total = bytes_do_buffer(m, x).1;
            let lv = m.arg(a, 2);
            let n = if lv == Valor::UNDEFINED {
                if total % tam != 0 {
                    return m.erro_alcance(&format!("byte length of {} should be a multiple of {tam}", t.nome()));
                }
                if desl > total {
                    return m.erro_alcance(&format!("Start offset {desl} is outside the bounds of the buffer"));
                }
                (total - desl) / tam
            } else {
                let n = para_indice(m, lv, "typed array length")?;
                if u64::from(desl) + u64::from(n) * u64::from(tam) > u64::from(total) {
                    return m.erro_alcance(&format!("Invalid typed array length: {n}"));
                }
                n
            };
            novo_tipado(m, t, x, desl, n, p)
        }
        Some(_) => {
            let itens = itens_de(m, x)?;
            let o = tipado_vazio(m, t, itens.len() as u32)?;
            for (i, v) in itens.into_iter().enumerate() {
                m.tipado_gravar(o, i as u32, v)?;
            }
            o
        }
    };
    if p != padrao {
        m.definir_proto(o, p);
    }
    Ok(o)
}

/// Itens de outro array tipado, de um iterável ou de um array-like.
fn itens_de(m: &mut Motor, x: Valor) -> Res<Vec<Valor>> {
    if classe(m, x) == Some(Classe::ArrayTipado) {
        let n = m.tipado_len(x);
        return Ok((0..n).filter_map(|i| m.tipado_ler(x, i)).collect());
    }
    let it = m.obter(x, ChaveProp::Nome(simb::ITERATOR))?;
    if !it.e_nulo_ou_indefinido() {
        return m.coletar_iteravel(x);
    }
    let n = m.comprimento_de(x)?;
    let mut v = Vec::with_capacity(n.min(1 << 20) as usize);
    for i in 0..n {
        v.push(m.obter_indice(x, i)?);
    }
    Ok(v)
}

fn tipado_de(m: &mut Motor, this: Valor, nome: &str) -> Res<(Tipo, u32)> {
    if classe(m, this) == Some(Classe::ArrayTipado) {
        return Ok((Tipo::de(m.interno(this, 1)), m.tipado_len(this)));
    }
    m.erro_tipo(&format!("this is not a typed array ({nome})"))
}

fn ta_buffer(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if matches!(classe(m, this), Some(Classe::ArrayTipado | Classe::DataView)) {
        return Ok(Valor(m.interno(this, 0)));
    }
    m.erro_tipo("this is not a typed array or DataView")
}

fn ta_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "byteLength")?;
    Ok(m.numero(f64::from(n) * f64::from(t.tamanho())))
}

fn ta_byte_offset(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if matches!(classe(m, this), Some(Classe::ArrayTipado | Classe::DataView)) {
        return Ok(m.numero(f64::from(m.interno(this, 2))));
    }
    m.erro_tipo("this is not a typed array or DataView")
}

fn ta_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = tipado_de(m, this, "length")?;
    Ok(m.numero(f64::from(n)))
}

fn ta_tag(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if classe(m, this) != Some(Classe::ArrayTipado) {
        return Ok(Valor::UNDEFINED);
    }
    let t = Tipo::de(m.interno(this, 1));
    Ok(m.texto(t.nome()))
}

/// Tipo do construtor `c` (um dos nossos, ou o do `prototype` dele).
fn tipo_do_construtor(m: &mut Motor, c: Valor) -> Res<Tipo> {
    let p = m.obter(c, ChaveProp::Nome(fixos::PROTOTYPE))?;
    for (i, &q) in m.intr.tipados_protos.iter().enumerate() {
        if q == p {
            return Ok(TIPOS[i].0);
        }
    }
    m.erro_tipo("not a typed array constructor")
}

fn ta_from(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = tipo_do_construtor(m, this)?;
    let (x, f, this_f) = (m.arg(a, 0), m.arg(a, 1), m.arg(a, 2));
    let itens = itens_de(m, x)?;
    let o = tipado_vazio(m, t, itens.len() as u32)?;
    for (i, mut v) in itens.into_iter().enumerate() {
        if f != Valor::UNDEFINED {
            let iv = Valor::int(i as i32);
            v = m.chamar(f, this_f, &[v, iv])?;
        }
        m.tipado_gravar(o, i as u32, v)?;
    }
    Ok(o)
}

fn ta_of(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = tipo_do_construtor(m, this)?;
    let o = tipado_vazio(m, t, a.n)?;
    for i in 0..a.n {
        let v = m.arg(a, i);
        m.tipado_gravar(o, i, v)?;
    }
    Ok(o)
}

/// `alvo.set(origem, deslocamento)`.
fn ta_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "set")?;
    let x = m.arg(a, 0);
    let desl = m.para_inteiro(m.arg(a, 1))?;
    if desl < 0.0 {
        return m.erro_alcance("offset is out of bounds");
    }
    // Mesmo tipo: cópia de bytes (certa também quando as duas vistas dividem o buffer).
    if classe(m, x) == Some(Classe::ArrayTipado) && Tipo::de(m.interno(x, 1)) == t {
        let k = m.tipado_len(x);
        if desl + f64::from(k) > f64::from(n) {
            return m.erro_alcance("offset is out of bounds");
        }
        let (_, de, _) = partes(m, x);
        let (_, para, _) = partes(m, this);
        let tam = t.tamanho();
        copiar_bytes(m, de, para + desl as u32 * tam, k * tam);
        return Ok(Valor::UNDEFINED);
    }
    let itens = if classe(m, x) == Some(Classe::ArrayTipado) {
        itens_de(m, x)?
    } else {
        let o = m.para_objeto(x)?;
        let k = m.comprimento_de(o)?;
        if desl + k as f64 > f64::from(n) {
            return m.erro_alcance("offset is out of bounds");
        }
        let mut v = Vec::with_capacity(k as usize);
        for i in 0..k {
            v.push(m.obter_indice(o, i)?);
        }
        v
    };
    if desl + itens.len() as f64 > f64::from(n) {
        return m.erro_alcance("offset is out of bounds");
    }
    for (i, v) in itens.into_iter().enumerate() {
        m.tipado_gravar(this, desl as u32 + i as u32, v)?;
    }
    Ok(Valor::UNDEFINED)
}

/// Nova vista sobre o mesmo buffer.
fn ta_subarray(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "subarray")?;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fim = relativo(m, m.arg(a, 1), n, n)?.max(ini);
    let buf = Valor(m.interno(this, 0));
    let desl = m.interno(this, 2) + ini * t.tamanho();
    let p = m.proto_de(this);
    Ok(novo_tipado(m, t, buf, desl, fim - ini, p))
}

/// Cópia de `ini..fim` num buffer novo.
fn ta_slice(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "slice")?;
    let ini = relativo(m, m.arg(a, 0), n, 0)?;
    let fim = relativo(m, m.arg(a, 1), n, n)?.max(ini);
    let o = tipado_vazio(m, t, fim - ini)?;
    let tam = t.tamanho();
    let (_, de, _) = partes(m, this);
    let (_, para, _) = partes(m, o);
    copiar_bytes(m, de + ini * tam, para, (fim - ini) * tam);
    Ok(o)
}

fn funcao(m: &mut Motor, f: Valor) -> Res<()> {
    if m.e_funcao(f) {
        return Ok(());
    }
    m.erro_tipo("callback is not a function")
}

fn ta_map(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "map")?;
    let (f, this_f) = (m.arg(a, 0), m.arg(a, 1));
    funcao(m, f)?;
    let o = tipado_vazio(m, t, n)?;
    for i in 0..n {
        let v = m.tipado_ler(this, i).unwrap_or(Valor::UNDEFINED);
        let iv = Valor::int(i as i32);
        let r = m.chamar(f, this_f, &[v, iv, this])?;
        m.tipado_gravar(o, i, r)?;
    }
    Ok(o)
}

fn ta_filter(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "filter")?;
    let (f, this_f) = (m.arg(a, 0), m.arg(a, 1));
    funcao(m, f)?;
    let mut ficam = Vec::new();
    for i in 0..n {
        let v = m.tipado_ler(this, i).unwrap_or(Valor::UNDEFINED);
        let iv = Valor::int(i as i32);
        let r = m.chamar(f, this_f, &[v, iv, this])?;
        if m.verdadeiro(r) {
            ficam.push(v);
        }
    }
    let o = tipado_vazio(m, t, ficam.len() as u32)?;
    for (i, v) in ficam.into_iter().enumerate() {
        m.tipado_gravar(o, i as u32, v)?;
    }
    Ok(o)
}

/// Itens em ordem (numérica, ou pelo comparador).
fn ordenados(m: &mut Motor, this: Valor, f: Valor) -> Res<Vec<Valor>> {
    if f != Valor::UNDEFINED {
        funcao(m, f)?;
    }
    let n = m.tipado_len(this);
    let mut v: Vec<Valor> = (0..n).filter_map(|i| m.tipado_ler(this, i)).collect();
    if f == Valor::UNDEFINED {
        let mut x: Vec<f64> = v.iter().map(|&a| m.f64_de(a)).collect();
        x.sort_by(|a, b| match (a.is_nan(), b.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            // -0 antes de +0.
            _ => a.partial_cmp(b).unwrap().then_with(|| b.is_sign_positive().cmp(&a.is_sign_positive()).reverse()),
        });
        v = x.into_iter().map(|a| m.numero(a)).collect();
    } else {
        super::array::ordenar(m, &mut v, f)?;
    }
    Ok(v)
}

fn ta_sort(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    tipado_de(m, this, "sort")?;
    let v = ordenados(m, this, m.arg(a, 0))?;
    for (i, x) in v.into_iter().enumerate() {
        m.tipado_gravar(this, i as u32, x)?;
    }
    Ok(this)
}

fn ta_to_sorted(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "toSorted")?;
    let v = ordenados(m, this, m.arg(a, 0))?;
    let o = tipado_vazio(m, t, n)?;
    for (i, x) in v.into_iter().enumerate() {
        m.tipado_gravar(o, i as u32, x)?;
    }
    Ok(o)
}

fn ta_to_reversed(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "toReversed")?;
    let o = tipado_vazio(m, t, n)?;
    for i in 0..n {
        let v = m.tipado_ler(this, n - 1 - i).unwrap_or(Valor::UNDEFINED);
        m.tipado_gravar(o, i, v)?;
    }
    Ok(o)
}

fn ta_with(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (t, n) = tipado_de(m, this, "with")?;
    let rel = m.para_inteiro(m.arg(a, 0))?;
    let i = if rel < 0.0 { f64::from(n) + rel } else { rel };
    let v = m.para_numero_valor(m.arg(a, 1))?;
    if !(0.0..f64::from(n)).contains(&i) {
        return m.erro_alcance("Invalid typed array index");
    }
    let o = tipado_vazio(m, t, n)?;
    let tam = t.tamanho();
    let (_, de, _) = partes(m, this);
    let (_, para, _) = partes(m, o);
    copiar_bytes(m, de, para, n * tam);
    m.tipado_gravar(o, i as u32, v)?;
    Ok(o)
}

// ---------------------------------------------------------------- DataView

fn dv_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt == Valor::UNDEFINED {
        return m.erro_tipo("Constructor DataView requires 'new'");
    }
    let buf = m.arg(a, 0);
    if classe(m, buf) != Some(Classe::ArrayBuffer) {
        return m.erro_tipo("First argument to DataView constructor must be an ArrayBuffer");
    }
    let total = bytes_do_buffer(m, buf).1;
    let desl = para_indice(m, m.arg(a, 1), "DataView offset")?;
    if desl > total {
        return m.erro_alcance(&format!("Start offset {desl} is outside the bounds of the buffer"));
    }
    let lv = m.arg(a, 2);
    let n = if lv == Valor::UNDEFINED { total - desl } else { para_indice(m, lv, "DataView length")? };
    if u64::from(desl) + u64::from(n) > u64::from(total) {
        return m.erro_alcance(&format!("Invalid DataView length {n}"));
    }
    let padrao = m.obter(m.chamado, ChaveProp::Nome(fixos::PROTOTYPE))?;
    let p = proto_de_nt(m, nt, padrao)?;
    let f = m.forma_para(Classe::DataView, p, 0);
    let o = m.novo_objeto(f);
    m.por_interno_valor(o, 0, buf);
    m.por_interno(o, 2, desl);
    m.por_interno(o, 3, n);
    Ok(o)
}

fn vista_de(m: &mut Motor, this: Valor) -> Res<(u32, u32)> {
    if classe(m, this) != Some(Classe::DataView) {
        return m.erro_tipo("this is not a DataView");
    }
    let buf = Valor(m.interno(this, 0));
    let (d, _) = bytes_do_buffer(m, buf);
    Ok((d + m.interno(this, 2), m.interno(this, 3)))
}

fn dv_byte_length(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let (_, n) = vista_de(m, this)?;
    Ok(m.numero(f64::from(n)))
}

/// Onde mora o item `t` no byte `pos` da vista (ou RangeError).
fn posicao(m: &mut Motor, this: Valor, pos: Valor, t: Tipo) -> Res<u32> {
    let (d, n) = vista_de(m, this)?;
    let i = para_indice(m, pos, "DataView index")?;
    if u64::from(i) + u64::from(t.tamanho()) > u64::from(n) {
        return m.erro_alcance("Offset is outside the bounds of the DataView");
    }
    Ok(d + i)
}

fn dv_get(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = Tipo::de(m.capturado().como_int() as u32);
    let d = posicao(m, this, m.arg(a, 0), t)?;
    let pequeno = m.verdadeiro(m.arg(a, 1));
    let x = t.ler(m.g.bytes(d, t.tamanho()), !pequeno);
    Ok(m.numero(x))
}

fn dv_set(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = Tipo::de(m.capturado().como_int() as u32);
    let pos = m.arg(a, 0);
    let x = m.para_numero(m.arg(a, 1))?;
    let d = posicao(m, this, pos, t)?;
    let pequeno = m.verdadeiro(m.arg(a, 2));
    t.gravar(m.g.bytes_mut(d, t.tamanho()), x, !pequeno);
    Ok(Valor::UNDEFINED)
}

#[cfg(test)]
mod testes {
    use super::Tipo;

    #[test]
    fn conversoes_dos_itens() {
        let mut b = [0u8; 8];
        Tipo::U8c.gravar(&mut b, 2.5, false);
        assert_eq!(b[0], 2);
        Tipo::U8c.gravar(&mut b, 3.5, false);
        assert_eq!(b[0], 4);
        Tipo::I8.gravar(&mut b, 200.0, false);
        assert_eq!(Tipo::I8.ler(&b, false), -56.0);
        Tipo::U32.gravar(&mut b, -1.0, false);
        assert_eq!(Tipo::U32.ler(&b, false), 4_294_967_295.0);
        Tipo::I16.gravar(&mut b, 258.0, true);
        assert_eq!(&b[..2], &[1, 2]);
        Tipo::F64.gravar(&mut b, 0.1, false);
        assert_eq!(Tipo::F64.ler(&b, false), 0.1);
    }
}
