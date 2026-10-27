//! JSON.parse e JSON.stringify (§25.5).

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::textos::ChaveProp;
use crate::valor::Valor;

pub(super) fn instalar(m: &mut Motor) {
    let j = m.objeto_vazio();
    m.metodo(j, "parse", 2, parse);
    m.metodo(j, "stringify", 3, stringify);
    let tag = m.texto("JSON");
    m.definir_propria(j, simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
    let g = m.intr.global;
    let c = m.chave("JSON");
    m.definir_propria(g, c, j, attr::METODO);
}

// ---------------------------------------------------------------- parse

struct Leitor<'a> {
    t: &'a [u16],
    i: usize,
}

impl Leitor<'_> {
    fn espacos(&mut self) {
        while self.i < self.t.len() && matches!(self.t[self.i], 0x20 | 0x09 | 0x0a | 0x0d) {
            self.i += 1;
        }
    }

    fn atual(&self) -> Option<u16> {
        self.t.get(self.i).copied()
    }
}

fn erro_sintaxe<T>(m: &mut Motor, l: &Leitor, msg: &str) -> Res<T> {
    let pos = l.i;
    Err(m.erro(TipoErro::Sintaxe, &format!("{msg} in JSON at position {pos}")))
}

fn valor(m: &mut Motor, l: &mut Leitor, fundo: u32) -> Res<Valor> {
    if fundo > 5000 {
        return erro_sintaxe(m, l, "Too deeply nested");
    }
    l.espacos();
    let Some(c) = l.atual() else { return erro_sintaxe(m, l, "Unexpected end of JSON input") };
    match c {
        0x7b => {
            l.i += 1;
            let o = m.objeto_vazio();
            l.espacos();
            if l.atual() == Some(0x7d) {
                l.i += 1;
                return Ok(o);
            }
            loop {
                l.espacos();
                if l.atual() != Some(0x22) {
                    return erro_sintaxe(m, l, "Expected property name or '}'");
                }
                let k = texto(m, l)?;
                l.espacos();
                if l.atual() != Some(0x3a) {
                    return erro_sintaxe(m, l, "Expected ':' after property name");
                }
                l.i += 1;
                let v = valor(m, l, fundo + 1)?;
                let k = m.chave_de_texto(k);
                m.criar_dado(o, k, v)?;
                l.espacos();
                match l.atual() {
                    Some(0x2c) => l.i += 1,
                    Some(0x7d) => {
                        l.i += 1;
                        return Ok(o);
                    }
                    _ => return erro_sintaxe(m, l, "Expected ',' or '}' after property value"),
                }
            }
        }
        0x5b => {
            l.i += 1;
            let a = m.array_de(&[]);
            l.espacos();
            if l.atual() == Some(0x5d) {
                l.i += 1;
                return Ok(a);
            }
            let mut n = 0;
            loop {
                let v = valor(m, l, fundo + 1)?;
                m.gravar_elemento(a, n, v);
                n += 1;
                l.espacos();
                match l.atual() {
                    Some(0x2c) => l.i += 1,
                    Some(0x5d) => {
                        l.i += 1;
                        return Ok(a);
                    }
                    _ => return erro_sintaxe(m, l, "Expected ',' or ']' after array element"),
                }
            }
        }
        0x22 => texto(m, l),
        0x74 | 0x66 | 0x6e => {
            for (palavra, v) in [("true", Valor::TRUE), ("false", Valor::FALSE), ("null", Valor::NULL)] {
                let p: Vec<u16> = palavra.encode_utf16().collect();
                if l.t[l.i..].starts_with(&p) {
                    l.i += p.len();
                    return Ok(v);
                }
            }
            erro_sintaxe(m, l, "Unexpected token")
        }
        0x2d | 0x30..=0x39 => numero(m, l),
        _ => erro_sintaxe(m, l, "Unexpected token"),
    }
}

fn numero(m: &mut Motor, l: &mut Leitor) -> Res<Valor> {
    let ini = l.i;
    let digitos = |l: &mut Leitor| {
        let i = l.i;
        while matches!(l.atual(), Some(0x30..=0x39)) {
            l.i += 1;
        }
        l.i - i
    };
    if l.atual() == Some(0x2d) {
        l.i += 1;
    }
    if l.atual() == Some(0x30) {
        l.i += 1;
    } else if digitos(l) == 0 {
        return erro_sintaxe(m, l, "No number after minus sign");
    }
    if l.atual() == Some(0x2e) {
        l.i += 1;
        if digitos(l) == 0 {
            return erro_sintaxe(m, l, "Unterminated fractional number");
        }
    }
    if matches!(l.atual(), Some(0x65 | 0x45)) {
        l.i += 1;
        if matches!(l.atual(), Some(0x2b | 0x2d)) {
            l.i += 1;
        }
        if digitos(l) == 0 {
            return erro_sintaxe(m, l, "Exponent part is missing a number");
        }
    }
    let s: String = l.t[ini..l.i].iter().map(|&c| c as u8 as char).collect();
    Ok(m.numero(s.parse::<f64>().unwrap_or(f64::NAN)))
}

fn texto(m: &mut Motor, l: &mut Leitor) -> Res<Valor> {
    l.i += 1;
    let ini = l.i;
    // Atalho: sem escapes, copia direto.
    while let Some(c) = l.atual() {
        if c == 0x22 {
            let v = m.texto_u16(&l.t[ini..l.i]);
            l.i += 1;
            return Ok(v);
        }
        if c == 0x5c || c < 0x20 {
            break;
        }
        l.i += 1;
    }
    let mut r: Vec<u16> = l.t[ini..l.i].to_vec();
    loop {
        let Some(c) = l.atual() else { return erro_sintaxe(m, l, "Unterminated string") };
        l.i += 1;
        match c {
            0x22 => return Ok(m.texto_u16(&r)),
            0x5c => {
                let Some(e) = l.atual() else { return erro_sintaxe(m, l, "Unterminated string") };
                l.i += 1;
                r.push(match e {
                    0x22 => 0x22,
                    0x5c => 0x5c,
                    0x2f => 0x2f,
                    0x62 => 0x08,
                    0x66 => 0x0c,
                    0x6e => 0x0a,
                    0x72 => 0x0d,
                    0x74 => 0x09,
                    0x75 => {
                        let h = l.t.get(l.i..l.i + 4).map(String::from_utf16_lossy).and_then(|s| u16::from_str_radix(&s, 16).ok());
                        let Some(h) = h else { return erro_sintaxe(m, l, "Bad Unicode escape") };
                        l.i += 4;
                        h
                    }
                    _ => return erro_sintaxe(m, l, "Bad escaped character"),
                });
            }
            c if c < 0x20 => return erro_sintaxe(m, l, "Bad control character in string literal"),
            c => r.push(c),
        }
    }
}

fn reviver(m: &mut Motor, f: Valor, holder: Valor, k: ChaveProp) -> Res<Valor> {
    let v = m.obter(holder, k)?;
    if m.e_objeto(v) {
        let ks = if m.e_array(v) {
            let n = m.comprimento_de(v)?;
            (0..n as u32).map(ChaveProp::Indice).collect()
        } else {
            m.chaves_de(v, true, true, false)
        };
        for kk in ks {
            let nv = reviver(m, f, v, kk)?;
            if nv == Valor::UNDEFINED {
                m.apagar(v, kk)?;
            } else {
                m.criar_dado(v, kk, nv)?;
            }
        }
    }
    let kv = m.chave_para_valor(k);
    m.chamar(f, holder, &[kv, v])
}

fn parse(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    let u = m.texto_u16_de(t);
    let mut l = Leitor { t: &u, i: 0 };
    let v = valor(m, &mut l, 0)?;
    l.espacos();
    if l.i < u.len() {
        return erro_sintaxe(m, &l, "Unexpected non-whitespace character after JSON");
    }
    let f = m.arg(a, 1);
    if m.e_funcao(f) {
        let raiz = m.objeto_vazio();
        m.criar_dado(raiz, ChaveProp::Nome(fixos::ANONIMO), v)?;
        return reviver(m, f, raiz, ChaveProp::Nome(fixos::ANONIMO));
    }
    Ok(v)
}

// ---------------------------------------------------------------- stringify

struct Escritor {
    saida: Vec<u16>,
    pilha: Vec<Valor>,
    recuo: Vec<u16>,
    espaco: Vec<u16>,
    substituto: Valor,
    lista: Option<Vec<ChaveProp>>,
}

fn citar(s: &mut Vec<u16>, t: &[u16]) {
    s.push(0x22);
    for (i, &c) in t.iter().enumerate() {
        match c {
            0x22 => s.extend_from_slice(&[0x5c, 0x22]),
            0x5c => s.extend_from_slice(&[0x5c, 0x5c]),
            0x08 => s.extend_from_slice(&[0x5c, 0x62]),
            0x0c => s.extend_from_slice(&[0x5c, 0x66]),
            0x0a => s.extend_from_slice(&[0x5c, 0x6e]),
            0x0d => s.extend_from_slice(&[0x5c, 0x72]),
            0x09 => s.extend_from_slice(&[0x5c, 0x74]),
            c if c < 0x20 || (0xd800..0xe000).contains(&c) && !par_valido(t, i) => {
                s.extend(format!("\\u{c:04x}").encode_utf16());
            }
            c => s.push(c),
        }
    }
    s.push(0x22);
}

fn par_valido(t: &[u16], i: usize) -> bool {
    let c = t[i];
    if (0xd800..0xdc00).contains(&c) {
        t.get(i + 1).is_some_and(|&d| (0xdc00..0xe000).contains(&d))
    } else {
        i > 0 && (0xd800..0xdc00).contains(&t[i - 1])
    }
}

/// Escreve `holder[k]`; devolve falso se o valor é "undefined" (não escreve nada).
fn serializar(m: &mut Motor, e: &mut Escritor, holder: Valor, k: ChaveProp, v: Valor) -> Res<bool> {
    let mut v = v;
    if m.e_objeto(v) || m.e_bigint(v) {
        let tj = m.obter(v, ChaveProp::Nome(fixos::TO_JSON))?;
        if m.e_funcao(tj) {
            let kv = m.chave_para_valor(k);
            v = m.chamar(tj, v, &[kv])?;
        }
    }
    if e.substituto != Valor::UNDEFINED {
        let kv = m.chave_para_valor(k);
        v = m.chamar(e.substituto, holder, &[kv, v])?;
    }
    if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Caixa {
        let p = Valor(m.interno(v, 0));
        if m.e_numero(p) {
            v = m.para_numero_valor(v)?;
        } else if m.e_texto(p) {
            v = m.para_texto(v)?;
        } else if p == Valor::TRUE || p == Valor::FALSE {
            v = p;
        }
    }
    match v {
        Valor::NULL => e.saida.extend("null".encode_utf16()),
        Valor::TRUE => e.saida.extend("true".encode_utf16()),
        Valor::FALSE => e.saida.extend("false".encode_utf16()),
        _ if m.e_texto(v) => {
            let u = m.texto_u16_de(v);
            citar(&mut e.saida, &u);
        }
        _ if m.e_numero(v) => {
            let x = m.f64_de(v);
            if x.is_finite() {
                let t = m.numero_para_texto(x);
                e.saida.extend(m.texto_u16_de(t));
            } else {
                e.saida.extend("null".encode_utf16());
            }
        }
        _ if m.e_bigint(v) => return m.erro_tipo("Do not know how to serialize a BigInt"),
        _ if m.e_objeto(v) && !m.e_funcao(v) => {
            if e.pilha.contains(&v) {
                return m.erro_tipo("Converting circular structure to JSON");
            }
            e.pilha.push(v);
            let antes = e.recuo.clone();
            e.recuo.extend_from_slice(&e.espaco.clone());
            let r = if eh_array(m, v) { array(m, e, v) } else { objeto(m, e, v) };
            e.recuo = antes;
            e.pilha.pop();
            r?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn eh_array(m: &mut Motor, mut v: Valor) -> bool {
    while m.formas.get(m.forma(v)).classe == Classe::Proxy {
        v = Valor(m.interno(v, 0));
    }
    m.e_array(v)
}

fn quebra(e: &mut Escritor) {
    if !e.espaco.is_empty() {
        e.saida.push(0x0a);
        let r = e.recuo.clone();
        e.saida.extend_from_slice(&r);
    }
}

fn objeto(m: &mut Motor, e: &mut Escritor, v: Valor) -> Res<()> {
    let ks = match &e.lista {
        Some(l) => l.clone(),
        None => m.chaves_de(v, true, true, false),
    };
    e.saida.push(0x7b);
    let mut algum = false;
    for k in ks {
        let x = m.obter(v, k)?;
        let marca = e.saida.len();
        if algum {
            e.saida.push(0x2c);
        }
        quebra(e);
        let kv = m.chave_para_valor(k);
        let ku = m.texto_u16_de(kv);
        citar(&mut e.saida, &ku);
        e.saida.push(0x3a);
        if !e.espaco.is_empty() {
            e.saida.push(0x20);
        }
        if serializar(m, e, v, k, x)? {
            algum = true;
        } else {
            e.saida.truncate(marca);
        }
    }
    if algum && !e.espaco.is_empty() {
        e.saida.push(0x0a);
        let n = e.recuo.len() - e.espaco.len();
        let r = e.recuo[..n].to_vec();
        e.saida.extend_from_slice(&r);
    }
    e.saida.push(0x7d);
    Ok(())
}

fn array(m: &mut Motor, e: &mut Escritor, v: Valor) -> Res<()> {
    let n = m.comprimento_de(v)?;
    e.saida.push(0x5b);
    for i in 0..n {
        if i > 0 {
            e.saida.push(0x2c);
        }
        quebra(e);
        let x = m.obter_indice(v, i)?;
        if !serializar(m, e, v, ChaveProp::Indice(i as u32), x)? {
            e.saida.extend("null".encode_utf16());
        }
    }
    if n > 0 && !e.espaco.is_empty() {
        e.saida.push(0x0a);
        let k = e.recuo.len() - e.espaco.len();
        let r = e.recuo[..k].to_vec();
        e.saida.extend_from_slice(&r);
    }
    e.saida.push(0x5d);
    Ok(())
}

fn stringify(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let (v, sub, esp) = (m.arg(a, 0), m.arg(a, 1), m.arg(a, 2));
    let mut e = Escritor { saida: Vec::new(), pilha: Vec::new(), recuo: Vec::new(), espaco: Vec::new(), substituto: Valor::UNDEFINED, lista: None };
    if m.e_funcao(sub) {
        e.substituto = sub;
    } else if m.e_objeto(sub) && m.e_array(sub) {
        let mut l = Vec::new();
        let n = m.comprimento_de(sub)?;
        for i in 0..n {
            let x = m.obter_indice(sub, i)?;
            let ok = m.e_texto(x) || m.e_numero(x) || (m.e_objeto(x) && m.formas.get(m.forma(x)).classe == Classe::Caixa);
            if ok {
                let k = m.para_chave(x)?;
                if !l.contains(&k) {
                    l.push(k);
                }
            }
        }
        e.lista = Some(l);
    }
    let esp = if m.e_objeto(esp) && m.formas.get(m.forma(esp)).classe == Classe::Caixa { Valor(m.interno(esp, 0)) } else { esp };
    if m.e_numero(esp) {
        let n = m.f64_de(esp).clamp(0.0, 10.0) as usize;
        e.espaco = vec![0x20; n];
    } else if m.e_texto(esp) {
        let mut u = m.texto_u16_de(esp);
        u.truncate(10);
        e.espaco = u;
    }
    let raiz = m.objeto_vazio();
    m.criar_dado(raiz, ChaveProp::Nome(fixos::ANONIMO), v)?;
    if serializar(m, &mut e, raiz, ChaveProp::Nome(fixos::ANONIMO), v)? {
        Ok(m.texto_u16(&e.saida))
    } else {
        Ok(Valor::UNDEFINED)
    }
}
