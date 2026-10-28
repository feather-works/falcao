//! RegExp (§22.2) sobre o motor de `crate::regex`.
//!
//! Campos internos: `[fonte][flags cru][programa cru]`; `lastIndex` é propriedade própria.

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::regex::{Flags, Programa};
use crate::textos::{ChaveProp, TEXTO_MAX};
use crate::valor::Valor;
use std::rc::Rc;

const G: u32 = 1;
const I: u32 = 2;
const M: u32 = 4;
const S: u32 = 8;
const U: u32 = 16;
const Y: u32 = 32;
const D: u32 = 64;
const V: u32 = 128;

pub(super) fn instalar(m: &mut Motor) {
    let op = m.intr.objeto_proto;
    let f = m.forma_para(Classe::Comum, op, 0);
    let p = m.novo_objeto(f);
    m.intr.regex_proto = p;
    let ctor = m.nativa("RegExp", 2, construtor, true);
    m.definir_propria(ctor, fixos::PROTOTYPE, p, 0);
    m.definir_propria(p, fixos::CONSTRUCTOR, ctor, attr::METODO);
    let g = m.intr.global;
    let c = m.chave("RegExp");
    m.definir_propria(g, c, ctor, attr::METODO);
    m.intr.regex_exec = m.metodo(p, "exec", 1, exec);
    m.metodo(p, "test", 1, test);
    m.metodo(p, "toString", 0, to_string);
    m.metodo(p, "compile", 2, compile);
    m.metodo(ctor, "escape", 1, escapar);
    let getters: &[(&str, FnNativa)] = &[
        ("source", get_source),
        ("flags", get_flags),
        ("global", get_global),
        ("ignoreCase", get_ignore_case),
        ("multiline", get_multiline),
        ("dotAll", get_dot_all),
        ("unicode", get_unicode),
        ("unicodeSets", get_unicode_sets),
        ("sticky", get_sticky),
        ("hasIndices", get_has_indices),
    ];
    for &(n, f) in getters {
        let gf = m.nativa(n, 0, f, false);
        let k = m.chave(n);
        m.definir_acessor(p, ChaveProp::Nome(k), gf, false, false);
    }
    for (s, n, f) in [
        (simb::MATCH, "[Symbol.match]", simbolo_match as FnNativa),
        (simb::MATCH_ALL, "[Symbol.matchAll]", simbolo_match_all),
        (simb::REPLACE, "[Symbol.replace]", simbolo_replace),
        (simb::SEARCH, "[Symbol.search]", simbolo_search),
        (simb::SPLIT, "[Symbol.split]", simbolo_split),
    ] {
        // length da norma: @@replace e @@split recebem 2 argumentos.
        let aridade = if s == simb::REPLACE || s == simb::SPLIT { 2 } else { 1 };
        let fv = m.nativa(n, aridade, f, false);
        m.definir_propria(p, s, fv, attr::METODO);
    }
    let sp = m.nativa("get [Symbol.species]", 0, especie, false);
    m.definir_acessor(ctor, ChaveProp::Nome(simb::SPECIES), sp, false, false);
}

fn especie(_m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    Ok(this)
}

// ---------------------------------------------------------------- criar

fn bits_de(f: &Flags) -> u32 {
    let mut b = 0;
    for (x, v) in [(f.global, G), (f.ignorar_caixa, I), (f.multilinha, M), (f.ponto_tudo, S), (f.unicode, U), (f.aderente, Y), (f.indices, D), (f.v, V)] {
        if x {
            b |= v;
        }
    }
    b
}

fn texto_flags(b: u32) -> String {
    let mut s = String::new();
    for (v, c) in [(D, 'd'), (G, 'g'), (I, 'i'), (M, 'm'), (S, 's'), (U, 'u'), (V, 'v'), (Y, 'y')] {
        if b & v != 0 {
            s.push(c);
        }
    }
    s
}

/// Compila (com cache) e inicia o objeto regex `o`.
fn iniciar(m: &mut Motor, o: Valor, fonte: Valor, flags: Valor) -> Res<()> {
    let fu = m.texto_u16_de(fonte);
    let flu = m.texto_u16_de(flags);
    let fl = match Flags::de_texto(&flu) {
        Ok(f) => f,
        Err(_) => {
            let s = String::from_utf16_lossy(&flu);
            return Err(m.erro(TipoErro::Sintaxe, &format!("Invalid flags supplied to RegExp constructor '{s}'")));
        }
    };
    let chave = (fu.clone(), flu);
    let idx = match m.cache_regex.get(&chave) {
        Some(&i) => i,
        None => match Programa::compilar(&fu, &fl) {
            Ok(p) => {
                m.regexes.push(Rc::new(p));
                let i = m.regexes.len() as u32 - 1;
                m.cache_regex.insert(chave, i);
                i
            }
            Err(e) => {
                let s = String::from_utf16_lossy(&fu);
                return Err(m.erro(TipoErro::Sintaxe, &format!("Invalid regular expression: /{s}/: {e}")));
            }
        },
    };
    let fonte = if m.texto_len(fonte) == 0 { m.texto("(?:)") } else { fonte };
    m.por_interno_valor(o, 0, fonte);
    m.por_interno(o, 1, bits_de(&fl));
    m.por_interno(o, 2, idx);
    let li = m.chave("lastIndex");
    m.definir_propria(o, li, Valor::int(0), attr::GRAVAVEL);
    Ok(())
}

fn novo_regex(m: &mut Motor, proto: Valor, fonte: Valor, flags: Valor) -> Res<Valor> {
    let f = m.forma_para(Classe::Regex, proto, 0);
    let o = m.novo_objeto(f);
    iniciar(m, o, fonte, flags)?;
    Ok(o)
}

impl Motor {
    /// `/corpo/flags` do código.
    pub(crate) fn criar_regex(&mut self, corpo: Valor, flags: Valor) -> Res<Valor> {
        let p = self.intr.regex_proto;
        novo_regex(self, p, corpo, flags)
    }

    pub fn e_regex(&self, v: Valor) -> bool {
        self.e_objeto(v) && self.formas.get(self.forma(v)).classe == Classe::Regex
    }
}

fn construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    let (p, f) = (m.arg(a, 0), m.arg(a, 1));
    let e_re = m.e_regex(p);
    if nt == Valor::UNDEFINED && e_re && f == Valor::UNDEFINED {
        let c = m.obter(p, ChaveProp::Nome(fixos::CONSTRUCTOR))?;
        if c == m.chamado {
            return Ok(p);
        }
    }
    let (fonte, flags) = if e_re {
        let fonte = Valor(m.interno(p, 0));
        let flags = if f == Valor::UNDEFINED {
            let b = m.interno(p, 1);
            m.texto(&texto_flags(b))
        } else {
            m.para_texto(f)?
        };
        (fonte, flags)
    } else {
        let fonte = if p == Valor::UNDEFINED { Valor::VAZIA } else { m.para_texto(p)? };
        let flags = if f == Valor::UNDEFINED { Valor::VAZIA } else { m.para_texto(f)? };
        (fonte, flags)
    };
    let proto = if nt == Valor::UNDEFINED {
        m.intr.regex_proto
    } else {
        let p = m.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
        if m.e_objeto(p) {
            p
        } else {
            m.intr.regex_proto
        }
    };
    novo_regex(m, proto, fonte, flags)
}

fn compile(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let re = este(m, this)?;
    let (p, f) = (m.arg(a, 0), m.arg(a, 1));
    let fonte = if p == Valor::UNDEFINED { Valor::VAZIA } else { m.para_texto(p)? };
    let flags = if f == Valor::UNDEFINED { Valor::VAZIA } else { m.para_texto(f)? };
    iniciar(m, re, fonte, flags)?;
    Ok(re)
}

fn este(m: &mut Motor, v: Valor) -> Res<Valor> {
    if m.e_regex(v) {
        Ok(v)
    } else {
        m.erro_tipo("RegExp method called on incompatible receiver")
    }
}

// ---------------------------------------------------------------- exec

fn ler_last_index(m: &mut Motor, re: Valor) -> Res<f64> {
    let v = m.obter(re, ChaveProp::Nome(fixos::LAST_INDEX))?;
    m.para_comprimento(v)
}

fn por_last_index(m: &mut Motor, re: Valor, i: f64) -> Res<()> {
    let li = fixos::LAST_INDEX;
    let v = m.numero(i);
    m.gravar(re, ChaveProp::Nome(li), v, true)
}

/// RegExpBuiltinExec: casamento cru (faixas) ou `None`.
fn casar(m: &mut Motor, re: Valor, s: &[u16]) -> Res<Option<Vec<Option<(usize, usize)>>>> {
    // Norma (§22.2.7.2): `lastIndex` primeiro (o valueOf dele pode chamar `compile`), depois as
    // flags e o programa.
    let global_ou_aderente = m.interno(re, 1) & (G | Y) != 0;
    let li = if global_ou_aderente {
        ler_last_index(m, re)?
    } else {
        // Sem g/y o valor não é usado: só a conversão, que só tem efeito (ou erro) se não é número.
        let v = m.obter(re, ChaveProp::Nome(fixos::LAST_INDEX))?;
        if !m.e_numero(v) {
            m.para_comprimento(v)?;
        }
        0.0
    };
    let b = m.interno(re, 1);
    let prog = Rc::clone(&m.regexes[m.interno(re, 2) as usize]);
    if li > s.len() as f64 {
        if global_ou_aderente {
            por_last_index(m, re, 0.0)?;
        }
        return Ok(None);
    }
    let r = if b & Y != 0 { prog.casar_em(s, li as usize) } else { prog.procurar(s, li as usize) };
    match r {
        None => {
            if global_ou_aderente {
                por_last_index(m, re, 0.0)?;
            }
            Ok(None)
        }
        Some(caps) => {
            if global_ou_aderente {
                let fim = caps[0].map_or(0, |c| c.1);
                por_last_index(m, re, fim as f64)?;
            }
            Ok(Some(caps))
        }
    }
}

/// Monta o array de resultado do exec.
fn resultado(m: &mut Motor, re: Valor, entrada: Valor, s: &[u16], caps: &[Option<(usize, usize)>]) -> Valor {
    let mut itens = Vec::with_capacity(caps.len());
    for c in caps {
        itens.push(match c {
            Some((i, j)) => m.texto_u16(&s[*i..*j]),
            None => Valor::UNDEFINED,
        });
    }
    let arr = m.array_de(&itens);
    let ini = caps[0].map_or(0, |c| c.0);
    let iv = m.numero(ini as f64);
    m.definir_propria(arr, fixos::INDEX, iv, attr::PADRAO);
    m.definir_propria(arr, fixos::INPUT, entrada, attr::PADRAO);
    let prog = Rc::clone(&m.regexes[m.interno(re, 2) as usize]);
    let grupos = if prog.nomes().is_empty() {
        Valor::UNDEFINED
    } else {
        let f = m.forma_para(Classe::Comum, Valor::NULL, 4);
        let g = m.novo_objeto(f);
        for (nome, i) in prog.nomes() {
            let k = m.nomes.internar(nome);
            let v = itens.get(*i).copied().unwrap_or(Valor::UNDEFINED);
            m.definir_propria(g, k, v, attr::PADRAO);
        }
        g
    };
    m.definir_propria(arr, fixos::GROUPS, grupos, attr::PADRAO);
    if m.interno(re, 1) & D != 0 {
        let mut pares = Vec::new();
        for c in caps {
            pares.push(match c {
                Some((i, j)) => {
                    let (a, b) = (m.numero(*i as f64), m.numero(*j as f64));
                    m.array_de(&[a, b])
                }
                None => Valor::UNDEFINED,
            });
        }
        let ind = m.array_de(&pares);
        // `indices.groups`: objeto com os pares dos grupos nomeados (ou undefined).
        let gi = if prog.nomes().is_empty() {
            Valor::UNDEFINED
        } else {
            let f = m.forma_para(Classe::Comum, Valor::NULL, 4);
            let g = m.novo_objeto(f);
            for (nome, i) in prog.nomes() {
                let k = m.nomes.internar(nome);
                let v = pares.get(*i).copied().unwrap_or(Valor::UNDEFINED);
                m.definir_propria(g, k, v, attr::PADRAO);
            }
            g
        };
        m.definir_propria(ind, fixos::GROUPS, gi, attr::PADRAO);
        let k = m.chave("indices");
        m.definir_propria(arr, k, ind, attr::PADRAO);
    }
    arr
}

fn exec(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let re = este(m, this)?;
    let t = m.para_texto(m.arg(a, 0))?;
    let s = m.texto_u16_de(t);
    match casar(m, re, &s)? {
        Some(caps) => Ok(resultado(m, re, t, &s, &caps)),
        None => Ok(Valor::NULL),
    }
}

/// RegExpExec: usa o `exec` do objeto se foi trocado. `s`: as unidades de `t` (quem chama em
/// laço converte uma vez; converter a cada casamento deixava `match`/`replace` com /g quadráticos).
fn regex_exec(m: &mut Motor, re: Valor, t: Valor, s: &[u16]) -> Res<Valor> {
    let k = m.chave("exec");
    let f = m.obter(re, ChaveProp::Nome(k))?;
    if m.e_funcao(f) && f != m.intr.regex_exec {
        let r = m.chamar(f, re, &[t])?;
        if !m.e_objeto(r) && r != Valor::NULL {
            return m.erro_tipo("exec result must be an object or null");
        }
        return Ok(r);
    }
    let re = este(m, re)?;
    match casar(m, re, s)? {
        Some(caps) => Ok(resultado(m, re, t, s, &caps)),
        None => Ok(Valor::NULL),
    }
}

fn test(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) {
        return m.erro_tipo("RegExp.prototype.test called on incompatible receiver");
    }
    let t = m.para_texto(m.arg(a, 0))?;
    // Atalho: sem montar o array de resultado.
    let k = m.chave("exec");
    let f = m.obter(this, ChaveProp::Nome(k))?;
    if m.e_regex(this) && f == m.intr.regex_exec {
        let s = m.texto_u16_de(t);
        return Ok(Valor::bool(casar(m, this, &s)?.is_some()));
    }
    let s = m.texto_u16_de(t);
    let r = regex_exec(m, this, t, &s)?;
    Ok(Valor::bool(r != Valor::NULL))
}

fn to_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) {
        return m.erro_tipo("RegExp.prototype.toString called on incompatible receiver");
    }
    let ks = m.chave("source");
    let kf = m.chave("flags");
    let s = m.obter(this, ChaveProp::Nome(ks))?;
    let s = m.para_texto_rust(s)?;
    let f = m.obter(this, ChaveProp::Nome(kf))?;
    let f = m.para_texto_rust(f)?;
    Ok(m.texto(&format!("/{s}/{f}")))
}

// ---------------------------------------------------------------- getters

fn bit(m: &mut Motor, this: Valor, b: u32) -> Res<Valor> {
    if this == m.intr.regex_proto {
        return Ok(Valor::UNDEFINED);
    }
    let re = este(m, this)?;
    Ok(Valor::bool(m.interno(re, 1) & b != 0))
}

fn get_global(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, G)
}
fn get_ignore_case(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, I)
}
fn get_multiline(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, M)
}
fn get_dot_all(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, S)
}
fn get_unicode(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, U)
}
fn get_unicode_sets(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, V)
}
fn get_sticky(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, Y)
}
fn get_has_indices(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bit(m, this, D)
}

fn get_source(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if this == m.intr.regex_proto {
        return Ok(m.texto("(?:)"));
    }
    let re = este(m, this)?;
    Ok(Valor(m.interno(re, 0)))
}

fn get_flags(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    if m.e_regex(this) {
        let b = m.interno(this, 1);
        return Ok(m.texto(&texto_flags(b)));
    }
    if !m.e_objeto(this) {
        return m.erro_tipo("RegExp.prototype.flags getter called on non-object");
    }
    let mut s = String::new();
    for (n, c) in [("hasIndices", 'd'), ("global", 'g'), ("ignoreCase", 'i'), ("multiline", 'm'), ("dotAll", 's'), ("unicode", 'u'), ("unicodeSets", 'v'), ("sticky", 'y')] {
        let k = m.chave(n);
        let v = m.obter(this, ChaveProp::Nome(k))?;
        if m.verdadeiro(v) {
            s.push(c);
        }
    }
    Ok(m.texto(&s))
}

// ---------------------------------------------------------------- símbolos

fn flags_de(m: &mut Motor, re: Valor) -> Res<u32> {
    if m.e_regex(re) {
        return Ok(m.interno(re, 1));
    }
    let k = m.chave("flags");
    let f = m.obter(re, ChaveProp::Nome(k))?;
    let f = m.para_texto_rust(f)?;
    Ok(f.chars().fold(0, |b, c| {
        b | match c {
            'g' => G,
            'u' => U,
            'v' => V,
            'y' => Y,
            _ => 0,
        }
    }))
}

/// Avança `i` um caractere (par substituto inteiro no modo unicode).
fn avancar(s: &[u16], i: usize, unicode: bool) -> usize {
    if unicode && i + 1 < s.len() && (0xd800..0xdc00).contains(&s[i]) && (0xdc00..0xe000).contains(&s[i + 1]) {
        i + 2
    } else {
        i + 1
    }
}

/// Atalho do laço global para um RegExp com o `exec` original e sem grupos nomeados: só as faixas,
/// sem montar um array de resultado por casamento. None = não serve, use `resultados`.
/// O `lastIndex` é próprio, de dados e não configurável: só as gravações do início e do fim se veem.
fn faixas_globais(m: &mut Motor, re: Valor, s: &[u16], unicode: bool) -> Res<Option<Vec<Vec<Option<(usize, usize)>>>>> {
    if !m.e_regex(re) {
        return Ok(None);
    }
    let k = m.chave("exec");
    if m.obter(re, ChaveProp::Nome(k))? != m.intr.regex_exec {
        return Ok(None);
    }
    let prog = Rc::clone(&m.regexes[m.interno(re, 2) as usize]);
    if !prog.nomes().is_empty() {
        return Ok(None);
    }
    let aderente = m.interno(re, 1) & Y != 0;
    por_last_index(m, re, 0.0)?;
    let mut r = Vec::new();
    let mut i = 0;
    while i <= s.len() {
        let Some(caps) = (if aderente { prog.casar_em(s, i) } else { prog.procurar(s, i) }) else { break };
        let (a, b) = caps[0].unwrap_or((i, i));
        i = if a == b { avancar(s, b, unicode) } else { b };
        r.push(caps);
    }
    por_last_index(m, re, 0.0)?;
    Ok(Some(r))
}

/// Todos os resultados (global) ou o primeiro.
fn resultados(m: &mut Motor, re: Valor, t: Valor, todos: bool) -> Res<Vec<Valor>> {
    let mut r = Vec::new();
    if !todos {
        let s = m.texto_u16_de(t);
        let x = regex_exec(m, re, t, &s)?;
        if x != Valor::NULL {
            r.push(x);
        }
        return Ok(r);
    }
    let unicode = flags_de(m, re)? & (U | V) != 0;
    por_last_index(m, re, 0.0)?;
    let s = m.texto_u16_de(t);
    loop {
        let x = regex_exec(m, re, t, &s)?;
        if x == Valor::NULL {
            break;
        }
        r.push(x);
        let c = m.obter(x, ChaveProp::Indice(0))?;
        let c = m.para_texto(c)?;
        if m.texto_len(c) == 0 {
            let li = ler_last_index(m, re)? as usize;
            por_last_index(m, re, avancar(&s, li, unicode) as f64)?;
        }
    }
    Ok(r)
}

fn simbolo_match(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    let fl = flags_de(m, this)?;
    let global = fl & G != 0;
    if global {
        let s = m.texto_u16_de(t);
        if let Some(fs) = faixas_globais(m, this, &s, fl & (U | V) != 0)? {
            if fs.is_empty() {
                return Ok(Valor::NULL);
            }
            let v: Vec<Valor> = fs.iter().map(|c| c[0].map_or(Valor::VAZIA, |(i, j)| m.texto_u16(&s[i..j]))).collect();
            return Ok(m.array_de(&v));
        }
    }
    let rs = resultados(m, this, t, global)?;
    if !global {
        return Ok(rs.first().copied().unwrap_or(Valor::NULL));
    }
    if rs.is_empty() {
        return Ok(Valor::NULL);
    }
    let mut v = Vec::new();
    for x in rs {
        let c = m.obter(x, ChaveProp::Indice(0))?;
        v.push(m.para_texto(c)?);
    }
    Ok(m.array_de(&v))
}

fn simbolo_match_all(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    // Cópia do regex, como manda a norma (o lastIndex do original não muda).
    let b = flags_de(m, this)?;
    let fonte = if m.e_regex(this) { Valor(m.interno(this, 0)) } else { Valor::VAZIA };
    let fl = m.texto(&texto_flags(b));
    let p = m.intr.regex_proto;
    let copia = novo_regex(m, p, fonte, fl)?;
    let li = ler_last_index(m, this)?;
    por_last_index(m, copia, li)?;
    // shortcut: calcula todos os resultados de uma vez (não é preguiçoso).
    let rs = resultados(m, copia, t, b & G != 0)?;
    let arr = m.array_de(&rs);
    let f = m.intr.array_valores;
    m.chamar(f, arr, &[])
}

/// Junta `u` ao texto em montagem sem passar do maior texto e sem abortar o processo se faltar
/// memória: `"1".repeat(2**20).replace(/(.+)/g, "$1".repeat(2**16))` pediria 2³⁶ unidades.
pub(crate) fn anexar(m: &mut Motor, r: &mut Vec<u16>, u: &[u16]) -> Res<()> {
    if r.len() + u.len() > TEXTO_MAX as usize {
        return m.erro_alcance("Invalid string length");
    }
    if r.capacity() - r.len() < u.len() && r.try_reserve(u.len()).is_err() {
        return m.erro_alcance("Out of memory");
    }
    r.extend_from_slice(u);
    Ok(())
}

/// GetSubstitution: entrega os pedaços do resultado a `r`, em ordem.
#[allow(clippy::too_many_arguments)]
fn substituir_em(m: &mut Motor, casado: &[u16], s: &[u16], pos: usize, caps: &[Valor], grupos: Valor, modelo: &[u16], r: &mut dyn FnMut(&mut Motor, &[u16]) -> Res<()>) -> Res<()> {
    let mut i = 0;
    let fim = (pos + casado.len()).min(s.len());
    while i < modelo.len() {
        let c = modelo[i];
        if c != b'$' as u16 || i + 1 >= modelo.len() {
            r(m, &[c])?;
            i += 1;
            continue;
        }
        let d = modelo[i + 1];
        match d {
            0x24 => {
                r(m, &[0x24])?;
                i += 2;
            }
            0x26 => {
                r(m, casado)?;
                i += 2;
            }
            0x60 => {
                r(m, &s[..pos.min(s.len())])?;
                i += 2;
            }
            0x27 => {
                r(m, &s[fim..])?;
                i += 2;
            }
            0x30..=0x39 => {
                let d1 = (d - 0x30) as usize;
                let dois = modelo.get(i + 2).filter(|x| (0x30..=0x39).contains(*x)).map(|x| d1 * 10 + (x - 0x30) as usize);
                let (n, largura) = match dois {
                    Some(n) if n >= 1 && n <= caps.len() => (n, 3),
                    _ => (d1, 2),
                };
                if n >= 1 && n <= caps.len() {
                    let v = caps[n - 1];
                    if v != Valor::UNDEFINED {
                        let t = m.para_texto(v)?;
                        let u = m.texto_u16_de(t);
                        r(m, &u)?;
                    }
                    i += largura;
                } else {
                    r(m, &[0x24])?;
                    i += 1;
                }
            }
            0x3c if grupos != Valor::UNDEFINED => {
                let Some(fecha) = modelo[i + 2..].iter().position(|&x| x == b'>' as u16) else {
                    r(m, &[0x24])?;
                    i += 1;
                    continue;
                };
                let nome = &modelo[i + 2..i + 2 + fecha];
                let k = m.nomes.internar(nome);
                let v = m.obter(grupos, ChaveProp::Nome(k))?;
                if v != Valor::UNDEFINED {
                    let t = m.para_texto(v)?;
                    let u = m.texto_u16_de(t);
                    r(m, &u)?;
                }
                i += 3 + fecha;
            }
            _ => {
                r(m, &[0x24])?;
                i += 1;
            }
        }
    }
    Ok(())
}

/// GetSubstitution (§22.1.3.19.1). O tamanho sai antes: um `$1` repetido pode pedir mais que o
/// maior texto, e aí é RangeError sem montar nada.
fn substituicao(m: &mut Motor, casado: &[u16], s: &[u16], pos: usize, caps: &[Valor], grupos: Valor, modelo: &[u16]) -> Res<Vec<u16>> {
    if !modelo.contains(&0x24) {
        return Ok(modelo.to_vec());
    }
    {
        let mut n = 0usize;
        substituir_em(m, casado, s, pos, caps, grupos, modelo, &mut |_, u| {
            n += u.len();
            Ok(())
        })?;
        if n > TEXTO_MAX as usize {
            return m.erro_alcance("Invalid string length");
        }
    }
    let mut r = Vec::new();
    substituir_em(m, casado, s, pos, caps, grupos, modelo, &mut |m, u| anexar(m, &mut r, u))?;
    Ok(r)
}

impl Motor {
    /// Substituição de `$…` para o `String.prototype.replace` com texto.
    pub fn substituicao_texto(&mut self, casado: &[u16], s: &[u16], pos: usize, modelo: &[u16]) -> Res<Vec<u16>> {
        substituicao(self, casado, s, pos, &[], Valor::UNDEFINED, modelo)
    }
}

fn simbolo_replace(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    let t = m.achatar_res(t)?;
    let s = m.texto_u16_de(t);
    let mut subst = m.arg(a, 1);
    let funcional = m.e_funcao(subst);
    if !funcional {
        subst = m.para_texto(subst)?;
    }
    let modelo = if funcional { Vec::new() } else { m.texto_u16_de(subst) };
    let fl = flags_de(m, this)?;
    let global = fl & G != 0;
    let mut saida = Vec::new();
    let mut prox = 0usize;
    if global {
        if let Some(fs) = faixas_globais(m, this, &s, fl & (U | V) != 0)? {
            for c in fs {
                let (pos, fim) = c[0].unwrap_or((0, 0));
                let caps: Vec<Valor> = c[1..].iter().map(|x| x.map_or(Valor::UNDEFINED, |(i, j)| m.texto_u16(&s[i..j]))).collect();
                let troca = if funcional {
                    let mut args = vec![m.texto_u16(&s[pos..fim])];
                    args.extend_from_slice(&caps);
                    args.push(m.numero(pos as f64));
                    args.push(t);
                    let r = m.chamar(subst, Valor::UNDEFINED, &args)?;
                    let r = m.para_texto(r)?;
                    m.texto_u16_de(r)
                } else {
                    substituicao(m, &s[pos..fim], &s, pos, &caps, Valor::UNDEFINED, &modelo)?
                };
                anexar(m, &mut saida, &s[prox..pos])?;
                anexar(m, &mut saida, &troca)?;
                prox = fim;
            }
            saida.extend_from_slice(&s[prox..]);
            return Ok(m.texto_u16(&saida));
        }
    }
    let rs = resultados(m, this, t, global)?;
    for x in rs {
        let n = m.comprimento_de(x)?;
        let c0 = m.obter(x, ChaveProp::Indice(0))?;
        let c0 = m.para_texto(c0)?;
        let casado = m.texto_u16_de(c0);
        let iv = m.obter(x, ChaveProp::Nome(fixos::INDEX))?;
        let pos = (m.para_inteiro(iv)?.max(0.0) as usize).min(s.len());
        let mut caps = Vec::new();
        for i in 1..n {
            let v = m.obter_indice(x, i)?;
            caps.push(if v == Valor::UNDEFINED { v } else { m.para_texto(v)? });
        }
        let grupos = m.obter(x, ChaveProp::Nome(fixos::GROUPS))?;
        let troca = if funcional {
            let mut args = vec![c0];
            args.extend_from_slice(&caps);
            args.push(m.numero(pos as f64));
            args.push(t);
            if grupos != Valor::UNDEFINED {
                args.push(grupos);
            }
            let r = m.chamar(subst, Valor::UNDEFINED, &args)?;
            let r = m.para_texto(r)?;
            m.texto_u16_de(r)
        } else {
            let g = if grupos == Valor::UNDEFINED { grupos } else { m.para_objeto(grupos)? };
            substituicao(m, &casado, &s, pos, &caps, g, &modelo)?
        };
        if pos >= prox {
            anexar(m, &mut saida, &s[prox..pos])?;
            anexar(m, &mut saida, &troca)?;
            prox = pos + casado.len();
        }
    }
    if prox < s.len() {
        saida.extend_from_slice(&s[prox..]);
    }
    Ok(m.texto_u16(&saida))
}

fn simbolo_search(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    let antes = ler_last_index(m, this)?;
    por_last_index(m, this, 0.0)?;
    let s = m.texto_u16_de(t);
    let r = regex_exec(m, this, t, &s)?;
    por_last_index(m, this, antes)?;
    if r == Valor::NULL {
        return Ok(Valor::int(-1));
    }
    m.obter(r, ChaveProp::Nome(fixos::INDEX))
}

/// RegExp.escape (ES2025): escapa uma string para ser casada literalmente.
fn escapar(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = m.arg(a, 0);
    if !m.e_texto(v) {
        return m.erro_tipo("RegExp.escape requires a string");
    }
    let s = m.texto_u16_de(v);
    let mut out: Vec<u16> = Vec::with_capacity(s.len() * 2);
    // `\xHH` (n = 2) ou `\uHHHH` (n = 4), em minúsculas.
    let hex = |out: &mut Vec<u16>, c: u32, n: usize| {
        out.push('\\' as u16);
        out.push(if n == 2 { 'x' } else { 'u' } as u16);
        for ch in format!("{:0w$x}", c, w = n).encode_utf16() {
            out.push(ch);
        }
    };
    let mut i = 0;
    while i < s.len() {
        let u = s[i] as u32;
        // Par substituto válido: copia os dois como estão.
        if (0xD800..0xDC00).contains(&u) && i + 1 < s.len() && (0xDC00..0xE000).contains(&(s[i + 1] as u32)) {
            out.push(s[i]);
            out.push(s[i + 1]);
            i += 2;
            continue;
        }
        let ch = char::from_u32(u).unwrap_or('\0');
        let primeiro = i == 0;
        if primeiro && ch.is_ascii_alphanumeric() {
            hex(&mut out, u, 2);
        } else if "^$\\.*+?()[]{}|/".contains(ch) {
            out.push('\\' as u16);
            out.push(s[i]);
        } else if let Some(e) = match ch {
            '\t' => Some('t'),
            '\n' => Some('n'),
            '\u{b}' => Some('v'),
            '\u{c}' => Some('f'),
            '\r' => Some('r'),
            _ => None,
        } {
            out.push('\\' as u16);
            out.push(e as u16);
        } else if ",-=<>#&!%:;@~'`\"".contains(ch) {
            hex(&mut out, u, 2);
        } else if u == 0x20 || u == 0xA0 {
            hex(&mut out, u, 2);
        } else if matches!(u, 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF)
            || (0xD800..0xE000).contains(&u)
        {
            hex(&mut out, u, 4);
        } else {
            out.push(s[i]);
        }
        i += 1;
    }
    Ok(m.texto_u16(&out))
}

/// SpeciesConstructor(o, %RegExp%) (§7.3.22).
fn construtor_da_especie(m: &mut Motor, o: Valor) -> Res<Valor> {
    let kc = ChaveProp::Nome(fixos::CONSTRUCTOR);
    let padrao = m.obter(m.intr.regex_proto, kc)?;
    let c = m.obter(o, kc)?;
    if c == Valor::UNDEFINED {
        return Ok(padrao);
    }
    if !m.e_objeto(c) {
        return m.erro_tipo("object.constructor is not an object");
    }
    let sp = m.obter(c, ChaveProp::Nome(simb::SPECIES))?;
    if sp == Valor::UNDEFINED || sp == Valor::NULL {
        return Ok(padrao);
    }
    if m.e_construtor(sp) {
        return Ok(sp);
    }
    m.erro_tipo("object.constructor[Symbol.species] is not a constructor")
}

/// RegExp.prototype[@@split] (§22.2.6.14): genérico, com o construtor da espécie e um
/// splitter sticky. Cada posição tenta um casamento colado em `q`.
fn simbolo_split(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    if !m.e_objeto(this) {
        return m.erro_tipo("RegExp.prototype[Symbol.split] called on non-object");
    }
    let t = m.para_texto(m.arg(a, 0))?;
    let c = construtor_da_especie(m, this)?;
    let kf = m.chave("flags");
    let fv = m.obter(this, ChaveProp::Nome(kf))?;
    let fv = m.para_texto(fv)?;
    let flags = String::from_utf16_lossy(&m.texto_u16_de(fv));
    let unicode = flags.contains('u') || flags.contains('v');
    let novas = if flags.contains('y') { flags } else { format!("{flags}y") };
    let nf = m.texto(&novas);
    let splitter = m.construir(c, &[this, nf], c)?;
    let s = m.texto_u16_de(t);
    let lim = m.arg(a, 1);
    let lim = if lim == Valor::UNDEFINED { u32::MAX } else { m.para_uint32(lim)? };
    let mut r: Vec<Valor> = Vec::new();
    if lim == 0 {
        return Ok(m.array_de(&r));
    }
    if s.is_empty() {
        if regex_exec(m, splitter, t, &s)? == Valor::NULL {
            r.push(t);
        }
        return Ok(m.array_de(&r));
    }
    let (mut p, mut q) = (0usize, 0usize);
    // Atalho: o splitter é um RegExp comum (protótipo e `exec` originais, aderente, mesmo modo
    // unicode). Ninguém mais vê o `lastIndex` dele, e as tentativas aderentes posição a posição
    // equivalem a procurar adiante: busca com o atalho do primeiro caractere, sem um objeto de
    // resultado por casamento.
    let ke = m.chave("exec");
    let comum = m.e_regex(splitter)
        && m.proto_de(splitter) == m.intr.regex_proto
        && m.interno(splitter, 1) & Y != 0
        && (m.interno(splitter, 1) & (U | V) != 0) == unicode
        && m.obter(splitter, ChaveProp::Nome(ke))? == m.intr.regex_exec;
    if comum {
        let prog = Rc::clone(&m.regexes[m.interno(splitter, 2) as usize]);
        while q < s.len() {
            let Some(caps) = prog.procurar_adiante(&s, q) else { break };
            let Some((ini, fim)) = caps[0] else { break };
            if ini >= s.len() {
                break;
            }
            let fim = fim.min(s.len());
            if fim == p {
                q = avancar(&s, ini, unicode);
                continue;
            }
            r.push(m.texto_u16(&s[p..ini]));
            if r.len() as u32 == lim {
                return Ok(m.array_de(&r));
            }
            p = fim;
            for c in &caps[1..] {
                r.push(c.map_or(Valor::UNDEFINED, |(i, j)| m.texto_u16(&s[i..j])));
                if r.len() as u32 == lim {
                    return Ok(m.array_de(&r));
                }
            }
            q = p;
        }
        r.push(m.texto_u16(&s[p..]));
        return Ok(m.array_de(&r));
    }
    let kl = m.chave("length");
    while q < s.len() {
        por_last_index(m, splitter, q as f64)?;
        let z = regex_exec(m, splitter, t, &s)?;
        if z == Valor::NULL {
            q = avancar(&s, q, unicode);
            continue;
        }
        let e = (ler_last_index(m, splitter)? as usize).min(s.len());
        if e == p {
            q = avancar(&s, q, unicode);
            continue;
        }
        let pedaco = m.texto_u16(&s[p..q]);
        r.push(pedaco);
        if r.len() as u32 == lim {
            return Ok(m.array_de(&r));
        }
        p = e;
        let n = m.obter(z, ChaveProp::Nome(kl))?;
        let n = (m.para_comprimento(n)? as usize).max(1) - 1;
        for i in 1..=n {
            let v = m.obter(z, ChaveProp::Indice(i as u32))?;
            r.push(v);
            if r.len() as u32 == lim {
                return Ok(m.array_de(&r));
            }
        }
        q = p;
    }
    let resto = m.texto_u16(&s[p..]);
    r.push(resto);
    Ok(m.array_de(&r))
}
