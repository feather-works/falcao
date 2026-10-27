//! RegExp (§22.2) sobre o motor de `crate::regex`.
//!
//! Campos internos: `[fonte][flags cru][programa cru]`; `lastIndex` é propriedade própria.

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, FnNativa, Res};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::regex::{Flags, Programa};
use crate::textos::ChaveProp;
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
        let fv = m.nativa(n, 1, f, false);
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
    let li = m.chave("lastIndex");
    let v = m.obter(re, ChaveProp::Nome(li))?;
    m.para_comprimento(v)
}

fn por_last_index(m: &mut Motor, re: Valor, i: f64) -> Res<()> {
    let li = m.chave("lastIndex");
    let v = m.numero(i);
    m.gravar(re, ChaveProp::Nome(li), v, true)
}

/// RegExpBuiltinExec: casamento cru (faixas) ou `None`.
fn casar(m: &mut Motor, re: Valor, s: &[u16]) -> Res<Option<Vec<Option<(usize, usize)>>>> {
    let b = m.interno(re, 1);
    let prog = Rc::clone(&m.regexes[m.interno(re, 2) as usize]);
    let global_ou_aderente = b & (G | Y) != 0;
    let li = if global_ou_aderente { ler_last_index(m, re)? } else { 0.0 };
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

/// RegExpExec: usa o `exec` do objeto se foi trocado.
fn regex_exec(m: &mut Motor, re: Valor, t: Valor) -> Res<Valor> {
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
    let s = m.texto_u16_de(t);
    match casar(m, re, &s)? {
        Some(caps) => Ok(resultado(m, re, t, &s, &caps)),
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
    let r = regex_exec(m, this, t)?;
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

/// Todos os resultados (global) ou o primeiro.
fn resultados(m: &mut Motor, re: Valor, t: Valor, todos: bool) -> Res<Vec<Valor>> {
    let mut r = Vec::new();
    if !todos {
        let x = regex_exec(m, re, t)?;
        if x != Valor::NULL {
            r.push(x);
        }
        return Ok(r);
    }
    let unicode = flags_de(m, re)? & (U | V) != 0;
    por_last_index(m, re, 0.0)?;
    let s = m.texto_u16_de(t);
    loop {
        let x = regex_exec(m, re, t)?;
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
    let global = flags_de(m, this)? & G != 0;
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

/// GetSubstitution (§22.1.3.19.1).
fn substituicao(m: &mut Motor, casado: &[u16], s: &[u16], pos: usize, caps: &[Valor], grupos: Valor, modelo: &[u16]) -> Res<Vec<u16>> {
    let mut r = Vec::new();
    let mut i = 0;
    let fim = (pos + casado.len()).min(s.len());
    while i < modelo.len() {
        let c = modelo[i];
        if c != b'$' as u16 || i + 1 >= modelo.len() {
            r.push(c);
            i += 1;
            continue;
        }
        let d = modelo[i + 1];
        match d {
            0x24 => {
                r.push(0x24);
                i += 2;
            }
            0x26 => {
                r.extend_from_slice(casado);
                i += 2;
            }
            0x60 => {
                r.extend_from_slice(&s[..pos.min(s.len())]);
                i += 2;
            }
            0x27 => {
                r.extend_from_slice(&s[fim..]);
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
                        r.extend(m.texto_u16_de(t));
                    }
                    i += largura;
                } else {
                    r.push(0x24);
                    i += 1;
                }
            }
            0x3c if grupos != Valor::UNDEFINED => {
                let Some(fecha) = modelo[i + 2..].iter().position(|&x| x == b'>' as u16) else {
                    r.push(0x24);
                    i += 1;
                    continue;
                };
                let nome = &modelo[i + 2..i + 2 + fecha];
                let k = m.nomes.internar(nome);
                let v = m.obter(grupos, ChaveProp::Nome(k))?;
                if v != Valor::UNDEFINED {
                    let t = m.para_texto(v)?;
                    r.extend(m.texto_u16_de(t));
                }
                i += 3 + fecha;
            }
            _ => {
                r.push(0x24);
                i += 1;
            }
        }
    }
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
    let s = m.texto_u16_de(t);
    let mut subst = m.arg(a, 1);
    let funcional = m.e_funcao(subst);
    if !funcional {
        subst = m.para_texto(subst)?;
    }
    let modelo = if funcional { Vec::new() } else { m.texto_u16_de(subst) };
    let global = flags_de(m, this)? & G != 0;
    let rs = resultados(m, this, t, global)?;
    let mut saida = Vec::new();
    let mut prox = 0usize;
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
            saida.extend_from_slice(&s[prox..pos]);
            saida.extend(troca);
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
    let r = regex_exec(m, this, t)?;
    por_last_index(m, this, antes)?;
    if r == Valor::NULL {
        return Ok(Valor::int(-1));
    }
    m.obter(r, ChaveProp::Nome(fixos::INDEX))
}

fn simbolo_split(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    let s = m.texto_u16_de(t);
    let lim = m.arg(a, 1);
    let lim = if lim == Valor::UNDEFINED { u32::MAX } else { m.para_uint32(lim)? };
    let re = este(m, this)?;
    let prog = Rc::clone(&m.regexes[m.interno(re, 2) as usize]);
    let unicode = m.interno(re, 1) & (U | V) != 0;
    let mut r: Vec<Valor> = Vec::new();
    if lim == 0 {
        return Ok(m.array_de(&r));
    }
    if s.is_empty() {
        if prog.casar_em(&s, 0).is_none() {
            r.push(t);
        }
        return Ok(m.array_de(&r));
    }
    let (mut p, mut q) = (0usize, 0usize);
    while q < s.len() {
        let Some(caps) = prog.procurar(&s, q) else { break };
        let (ini, fim) = caps[0].unwrap_or((s.len(), s.len()));
        if ini >= s.len() {
            break;
        }
        if fim == p || (fim == ini && ini == p) {
            q = avancar(&s, ini.max(q), unicode);
            continue;
        }
        let pedaco = m.texto_u16(&s[p..ini]);
        r.push(pedaco);
        if r.len() as u32 == lim {
            return Ok(m.array_de(&r));
        }
        for c in &caps[1..] {
            let v = match c {
                Some((i, j)) => m.texto_u16(&s[*i..*j]),
                None => Valor::UNDEFINED,
            };
            r.push(v);
            if r.len() as u32 == lim {
                return Ok(m.array_de(&r));
            }
        }
        p = fim;
        q = if fim == ini { avancar(&s, fim, unicode) } else { fim };
    }
    let resto = m.texto_u16(&s[p..]);
    r.push(resto);
    Ok(m.array_de(&r))
}
