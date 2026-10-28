//! JSON.parse e JSON.stringify (§25.5).

use crate::formas::{self, attr, Classe};
use crate::funcoes::{Args, Res};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::numeros;
use crate::textos::{indice_de_array, ChaveProp, Unidades, TEXTO_MAX};
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

/// Unidade do texto de entrada: bytes (texto Latin1, o caso comum) ou UTF-16. O leitor é o
/// mesmo; só a criação de textos e de chaves muda.
trait Unidade: Copy + PartialEq {
    fn u(self) -> u16;
    fn texto(m: &mut Motor, s: &[Self]) -> Valor;
    /// O nome (ou índice) da chave `s`, sem criar texto na gaiola.
    fn chave(m: &mut Motor, s: &[Self]) -> ChaveProp;
}

impl Unidade for u16 {
    fn u(self) -> u16 {
        self
    }
    fn texto(m: &mut Motor, s: &[u16]) -> Valor {
        m.texto_u16(s)
    }
    fn chave(m: &mut Motor, s: &[u16]) -> ChaveProp {
        match indice_de_array(s) {
            Some(i) => ChaveProp::Indice(i),
            None => ChaveProp::Nome(m.nomes.internar(s)),
        }
    }
}

impl Unidade for u8 {
    fn u(self) -> u16 {
        u16::from(self)
    }
    fn texto(m: &mut Motor, s: &[u8]) -> Valor {
        m.texto_latin1(s)
    }
    fn chave(m: &mut Motor, s: &[u8]) -> ChaveProp {
        let mut area = [0u16; 64];
        let largo: Vec<u16>;
        let u: &[u16] = if s.len() <= area.len() {
            for (d, &b) in area.iter_mut().zip(s) {
                *d = u16::from(b);
            }
            &area[..s.len()]
        } else {
            largo = s.iter().map(|&b| u16::from(b)).collect();
            &largo
        };
        u16::chave(m, u)
    }
}

struct Leitor<'a, U> {
    t: &'a [U],
    i: usize,
    /// Por (forma de origem, primeira unidade da chave), a última propriedade acrescentada:
    /// (chave, forma nova, campo). Arrays de objetos parecidos repetem as mesmas chaves na mesma
    /// ordem; a primeira unidade separa os objetos aninhados que saem da mesma forma vazia.
    pistas: falcao_sintaxe::atomos::MapaRapido<u64, (crate::formas::Chave, crate::formas::FormaId, u32)>,
    itens: Vec<Valor>,
}

impl<U: Unidade> Leitor<'_, U> {
    fn espacos(&mut self) {
        while self.i < self.t.len() && matches!(self.t[self.i].u(), 0x20 | 0x09 | 0x0a | 0x0d) {
            self.i += 1;
        }
    }

    fn atual(&self) -> Option<u16> {
        self.t.get(self.i).map(|c| c.u())
    }

    fn em(&self, i: usize) -> Option<u16> {
        self.t.get(i).map(|c| c.u())
    }
}

fn erro_sintaxe<T, U>(m: &mut Motor, l: &Leitor<U>, msg: &str) -> Res<T> {
    let pos = l.i;
    Err(m.erro(TipoErro::Sintaxe, &format!("{msg} in JSON at position {pos}")))
}

fn valor<U: Unidade>(m: &mut Motor, l: &mut Leitor<U>, fundo: u32) -> Res<Valor> {
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
                // Pista: o objeto está na forma de onde já saiu esta mesma chave (texto igual, sem
                // escape): segue a mesma transição sem internar nem procurar.
                let de = m.forma(o);
                let p = pista(de, l.em(l.i + 1));
                if let Some(&(c, para, slot)) = l.pistas.get(&p) {
                    let nome = m.nomes.texto(c);
                    let fim = l.i + 1 + nome.len();
                    let igual = l.t.get(l.i + 1..fim).is_some_and(|s| s.iter().zip(nome).all(|(a, &b)| a.u() == b));
                    if igual && l.em(fim) == Some(0x22) {
                        l.i = fim + 1;
                        l.espacos();
                        if l.atual() != Some(0x3a) {
                            return erro_sintaxe(m, l, "Expected ':' after property name");
                        }
                        l.i += 1;
                        let v = valor(m, l, fundo + 1)?;
                        // `valor` não mexe em `o`: a forma ainda é `de`.
                        m.g.por_u32(o.deslocamento(), para.0);
                        m.gravar_campo(o, slot, v);
                        l.espacos();
                        match l.atual() {
                            Some(0x2c) => {
                                l.i += 1;
                                continue;
                            }
                            Some(0x7d) => {
                                l.i += 1;
                                return Ok(o);
                            }
                            _ => return erro_sintaxe(m, l, "Expected ',' or '}' after property value"),
                        }
                    }
                }
                let k = chave(m, l)?;
                l.espacos();
                if l.atual() != Some(0x3a) {
                    return erro_sintaxe(m, l, "Expected ':' after property name");
                }
                l.i += 1;
                let v = valor(m, l, fundo + 1)?;
                match k {
                    // Objeto novo e comum, só com propriedades de dados padrão: uma busca só.
                    ChaveProp::Nome(c) => match m.propria(o, c) {
                        Some(d) => m.gravar_campo(o, d.slot, v),
                        None => {
                            let de = m.forma(o);
                            m.adicionar_propriedade(o, c, v, attr::PADRAO);
                            let para = m.forma(o);
                            if !m.formas.get(para).dicionario {
                                let slot = m.formas.procurar(para, c).unwrap().slot;
                                let primeira = m.nomes.texto(c).first().copied();
                                l.pistas.insert(pista(de, primeira), (c, para, slot));
                            }
                        }
                    },
                    k => {
                        m.criar_dado(o, k, v)?;
                    }
                }
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
            l.espacos();
            if l.atual() == Some(0x5d) {
                l.i += 1;
                return Ok(m.array_de(&[]));
            }
            // Os itens se juntam numa pilha compartilhada e o array nasce com o tamanho certo.
            let marca = l.itens.len();
            loop {
                let v = valor(m, l, fundo + 1)?;
                l.itens.push(v);
                l.espacos();
                match l.atual() {
                    Some(0x2c) => l.i += 1,
                    Some(0x5d) => {
                        l.i += 1;
                        let a = m.array_de(&l.itens[marca..]);
                        l.itens.truncate(marca);
                        return Ok(a);
                    }
                    _ => return erro_sintaxe(m, l, "Expected ',' or ']' after array element"),
                }
            }
        }
        0x22 => texto(m, l),
        0x74 | 0x66 | 0x6e => {
            for (palavra, v) in [(&b"true"[..], Valor::TRUE), (b"false", Valor::FALSE), (b"null", Valor::NULL)] {
                if l.t.get(l.i..l.i + palavra.len()).is_some_and(|u| u.iter().zip(palavra).all(|(a, &b)| a.u() == u16::from(b))) {
                    l.i += palavra.len();
                    return Ok(v);
                }
            }
            erro_sintaxe(m, l, "Unexpected token")
        }
        0x2d | 0x30..=0x39 => numero(m, l),
        _ => erro_sintaxe(m, l, "Unexpected token"),
    }
}

fn pista(de: crate::formas::FormaId, primeira: Option<u16>) -> u64 {
    u64::from(de.0) << 17 | primeira.map_or(1 << 16, u64::from)
}

fn numero<U: Unidade>(m: &mut Motor, l: &mut Leitor<U>) -> Res<Valor> {
    // Inteiro de até 9 dígitos (o caso comum): lê e converte numa passada só. Zero à esquerda,
    // `-0`, fração e expoente seguem pelo caminho geral, que também dá os erros.
    let r = &l.t[l.i..];
    let neg = r.first().is_some_and(|c| c.u() == 0x2d);
    let mut j = usize::from(neg);
    let mut n = 0i64;
    while let Some(d) = r.get(j).map(|c| c.u().wrapping_sub(0x30)).filter(|&d| d < 10) {
        n = n * 10 + i64::from(d);
        j += 1;
    }
    let nd = j - usize::from(neg);
    let fim_ok = !matches!(r.get(j).map(|c| c.u()), Some(0x2e | 0x65 | 0x45));
    if (1..=9).contains(&nd) && fim_ok && !(nd > 1 && r[usize::from(neg)].u() == 0x30) && !(neg && n == 0) {
        l.i += j;
        return Ok(m.numero(if neg { -n } else { n } as f64));
    }
    let ini = l.i;
    let digitos = |l: &mut Leitor<U>| {
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
    let u = &l.t[ini..l.i];
    // Inteiro curto (o caso comum): sem passar por texto. `-0` fica para o caminho geral.
    let (neg, ds) = match u.split_first() {
        Some((c, r)) if c.u() == 0x2d => (true, r),
        _ => (false, u),
    };
    if ds.len() <= 9 && ds.iter().all(|c| (0x30..=0x39).contains(&c.u())) && !(neg && ds.len() == 1 && ds[0].u() == 0x30) {
        let n = ds.iter().fold(0i64, |n, c| n * 10 + i64::from(c.u() - 0x30));
        return Ok(m.numero(if neg { -n } else { n } as f64));
    }
    // Só ASCII aqui (o leitor acima conferiu): uma área na pilha basta, sem `String`.
    let mut area = [0u8; 64];
    let x = if u.len() <= area.len() {
        for (d, c) in area.iter_mut().zip(u) {
            *d = c.u() as u8;
        }
        std::str::from_utf8(&area[..u.len()]).ok().and_then(|s| s.parse::<f64>().ok())
    } else {
        u.iter().map(|c| c.u() as u8 as char).collect::<String>().parse::<f64>().ok()
    };
    Ok(m.numero(x.unwrap_or(f64::NAN)))
}

/// Nome de propriedade: sem escape (quase sempre), interna direto das unidades do JSON, sem
/// criar o texto na gaiola.
fn chave<U: Unidade>(m: &mut Motor, l: &mut Leitor<U>) -> Res<ChaveProp> {
    let ini = l.i + 1;
    if let Some(n) = l.t[ini..].iter().position(|c| matches!(c.u(), 0x22 | 0x5c) || c.u() < 0x20) {
        if l.t[ini + n].u() == 0x22 {
            l.i = ini + n + 1;
            return Ok(U::chave(m, &l.t[ini..ini + n]));
        }
    }
    let k = texto(m, l)?;
    Ok(m.chave_de_texto(k))
}

fn texto<U: Unidade>(m: &mut Motor, l: &mut Leitor<U>) -> Res<Valor> {
    l.i += 1;
    let ini = l.i;
    // Atalho: sem escapes, copia direto.
    while let Some(c) = l.atual() {
        if c == 0x22 {
            let v = U::texto(m, &l.t[ini..l.i]);
            l.i += 1;
            return Ok(v);
        }
        if c == 0x5c || c < 0x20 {
            break;
        }
        l.i += 1;
    }
    let mut r: Vec<u16> = l.t[ini..l.i].iter().map(|c| c.u()).collect();
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
                        let h = l.t.get(l.i..l.i + 4).and_then(|s| s.iter().try_fold(0u16, |n, c| Some(n * 16 + char::from_u32(u32::from(c.u()))?.to_digit(16)? as u16)));
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
    // O reviver pode criar ciclo (`this[1] = o` com o apontando para si): recursão sem fim.
    m.checar_pilha()?;
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

/// O texto inteiro, do começo ao fim (sem nada sobrando além de espaços).
fn ler_tudo<U: Unidade>(m: &mut Motor, t: &[U]) -> Res<Valor> {
    let mut l = Leitor { t, i: 0, pistas: Default::default(), itens: Vec::new() };
    // Texto grande: o resultado nasce todo de uma vez e fica vivo, então nasce na geração velha.
    m.g.direto_na_velha(t.len() > 64 * 1024);
    let v = valor(m, &mut l, 0);
    m.g.direto_na_velha(false);
    let v = v?;
    l.espacos();
    if l.i < t.len() {
        return erro_sintaxe(m, &l, "Unexpected non-whitespace character after JSON");
    }
    Ok(v)
}

fn parse(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let t = m.para_texto(m.arg(a, 0))?;
    // Latin1 lido como bytes: cópia de 1 byte por caractere, sem alargar para UTF-16.
    let p = m.achatar(t);
    let latin1 = match m.unidades_planas(p) {
        Unidades::Latin1(b) => Some(b.to_vec()),
        Unidades::Utf16(_) => None,
    };
    let v = match latin1 {
        Some(b) => ler_tudo(m, &b)?,
        None => {
            let u = m.texto_u16_de(t);
            ler_tudo(m, &u)?
        }
    };
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
    /// Cada nome já citado (`"nome"`), como faixa em `citadas`: as mesmas chaves se repetem.
    nomes: falcao_sintaxe::atomos::MapaRapido<crate::formas::Chave, (u32, u32)>,
    citadas: Vec<u16>,
    /// Formas cuja cadeia (só objetos comuns) não tem `toJSON`. Vale enquanto nenhum código JS
    /// roda: todo caminho que pode rodar (getter, `toJSON`, proxy, caixa) a esvazia.
    sem_tojson: Vec<crate::formas::FormaId>,
    /// Plano de cada forma comum já vista: os campos enumeráveis em ordem, cada um com o nome
    /// já citado em `citadas`. Forma que não é dicionário não muda, então vale a chamada toda.
    planos: falcao_sintaxe::atomos::MapaRapido<u32, (u32, u32)>,
    campos: Vec<Campo>,
}

#[derive(Clone, Copy)]
struct Campo {
    chave: crate::formas::Chave,
    slot: u32,
    acessor: bool,
    /// Faixa de `"nome"` em `citadas`.
    a: u32,
    b: u32,
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

/// Como [`citar`], para texto só com unidades até 255 (não tem substituto solto).
fn citar_latin1(s: &mut Vec<u16>, t: &[u8]) {
    // Sem nada a escapar (quase sempre): copia o trecho de uma vez.
    if !t.iter().any(|&c| c < 0x20 || c == 0x22 || c == 0x5c) {
        s.reserve(t.len() + 2);
        s.push(0x22);
        s.extend(t.iter().map(|&c| u16::from(c)));
        s.push(0x22);
        return;
    }
    s.push(0x22);
    for &c in t {
        match c {
            0x22 | 0x5c | 0x08 | 0x0c | 0x0a | 0x0d | 0x09 => {
                let l = match c {
                    0x08 => b'b',
                    0x0c => b'f',
                    0x0a => b'n',
                    0x0d => b'r',
                    0x09 => b't',
                    c => c,
                };
                s.extend_from_slice(&[0x5c, u16::from(l)]);
            }
            c if c < 0x20 => s.extend(format!("\\u{c:04x}").encode_utf16()),
            c => s.push(u16::from(c)),
        }
    }
    s.push(0x22);
}

/// Cita o texto `t` lendo as unidades direto da gaiola (sem copiar para um `Vec`).
fn citar_texto(m: &mut Motor, s: &mut Vec<u16>, t: Valor) -> Res<()> {
    let p = m.achatar_res(t)?;
    // Texto grande: o tamanho citado (até 6× o original) sai antes; passar do maior texto ou
    // faltar memória vira RangeError em vez de derrubar o processo.
    if m.texto_len(p) >= 1 << 16 {
        let extra = |c: u16| match c {
            0x22 | 0x5c | 0x08 | 0x0c | 0x0a | 0x0d | 0x09 => 1,
            c if c < 0x20 => 5,
            _ => 0,
        };
        let n = match m.unidades_planas(p) {
            Unidades::Latin1(b) => b.len() + b.iter().map(|&c| extra(u16::from(c))).sum::<usize>(),
            Unidades::Utf16(u) => u.len() + u.iter().enumerate().map(|(i, &c)| if (0xd800..0xe000).contains(&c) && !par_valido(u, i) { 5 } else { extra(c) }).sum::<usize>(),
        } + 2;
        if s.len() + n > TEXTO_MAX as usize {
            return m.erro_alcance("Invalid string length");
        }
        if s.try_reserve(n).is_err() {
            return m.erro_alcance("Out of memory");
        }
    }
    match m.unidades_planas(p) {
        Unidades::Latin1(b) => citar_latin1(s, b),
        Unidades::Utf16(u) => citar(s, u),
    }
    Ok(())
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
    if folha(m, e, v) {
        return Ok(true);
    }
    m.checar_pilha()?;
    let mut v = v;
    if m.e_objeto(v) || m.e_bigint(v) {
        let f = m.forma(v);
        if !e.sem_tojson.contains(&f) {
            if cadeia_comum(m, v) && matches!(m.procurar(v, fixos::TO_JSON), crate::objetos::Achado::Ausente) {
                e.sem_tojson.push(f);
            } else {
                e.sem_tojson.clear();
                let tj = m.obter(v, ChaveProp::Nome(fixos::TO_JSON))?;
                if m.e_funcao(tj) {
                    let kv = m.chave_para_valor(k);
                    v = m.chamar(tj, v, &[kv])?;
                }
            }
        }
    }
    if e.substituto != Valor::UNDEFINED {
        let kv = m.chave_para_valor(k);
        v = m.chamar(e.substituto, holder, &[kv, v])?;
    }
    if m.e_objeto(v) && m.formas.get(m.forma(v)).classe == Classe::Caixa {
        e.sem_tojson.clear();
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
        _ if m.e_texto(v) => citar_texto(m, &mut e.saida, v)?,
        _ if v.e_int() => escrever_int(e, v.como_int()),
        _ if m.e_numero(v) => escrever_numero(e, m.f64_de(v)),
        _ if m.e_bigint(v) => return m.erro_tipo("Do not know how to serialize a BigInt"),
        _ if m.e_objeto(v) && !m.e_funcao(v) => {
            if e.pilha.contains(&v) {
                return m.erro_tipo("Converting circular structure to JSON");
            }
            e.pilha.push(v);
            let antes = e.recuo.len();
            let espaco = std::mem::take(&mut e.espaco);
            e.recuo.extend_from_slice(&espaco);
            e.espaco = espaco;
            let r = if eh_array(m, v) { array(m, e, v) } else { objeto(m, e, v) };
            e.recuo.truncate(antes);
            e.pilha.pop();
            r?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Primitivos sem substituto (a maioria das folhas): sem `toJSON`, sem caixa, um teste de forma
/// só. Falso = não é folha simples; segue o caminho completo de `serializar`.
#[inline(always)]
fn folha(m: &mut Motor, e: &mut Escritor, v: Valor) -> bool {
    if e.substituto != Valor::UNDEFINED {
        return false;
    }
    if v.e_int() {
        escrever_int(e, v.como_int());
        return true;
    }
    match v {
        Valor::NULL => e.saida.extend_from_slice(&[0x6e, 0x75, 0x6c, 0x6c]),
        Valor::TRUE => e.saida.extend_from_slice(&[0x74, 0x72, 0x75, 0x65]),
        Valor::FALSE => e.saida.extend_from_slice(&[0x66, 0x61, 0x6c, 0x73, 0x65]),
        _ if !v.e_ref() => return false,
        _ => {
            let f = m.forma(v);
            if (f == formas::LATIN1 || f == formas::UTF16 || f == formas::CORDA) && m.texto_len(v) < 1 << 16 {
                let _ = citar_texto(m, &mut e.saida, v);
            } else if f == formas::NUMERO {
                escrever_numero(e, m.f64_de(v));
            } else {
                return false;
            }
        }
    }
    true
}

fn escrever_int(e: &mut Escritor, i: i32) {
    let mut area = [0u16; 11];
    let mut p = area.len();
    let mut n = i.unsigned_abs();
    loop {
        p -= 1;
        area[p] = 0x30 + (n % 10) as u16;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if i < 0 {
        p -= 1;
        area[p] = 0x2d;
    }
    e.saida.extend_from_slice(&area[p..]);
}

fn escrever_numero(e: &mut Escritor, x: f64) {
    if x.is_finite() {
        numeros::escrever_numero(x, &mut |b| e.saida.extend(b.iter().map(|&c| u16::from(c))));
    } else {
        e.saida.extend("null".encode_utf16());
    }
}

/// O objeto e os protótipos são todos comuns (sem proxy nem exótico): `procurar` responde sem
/// rodar código.
fn cadeia_comum(m: &Motor, mut v: Valor) -> bool {
    for _ in 0..8 {
        if v == Valor::NULL {
            return true;
        }
        if !m.e_objeto(v) || !matches!(m.formas.get(m.forma(v)).classe, Classe::Comum | Classe::Array) {
            return false;
        }
        v = m.proto_de(v);
    }
    false
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
        e.saida.extend_from_slice(&e.recuo);
    }
}

/// Objeto comum só com nomes (sem índices, nem forma de dicionário): as chaves saem do plano da
/// forma, e enquanto a forma não muda o valor sai direto do campo (a mesma ordem e o mesmo valor
/// que `chaves_de` + `obter` dariam). Devolve a faixa do plano em `e.campos`.
fn plano(m: &mut Motor, e: &mut Escritor, v: Valor) -> Option<(u32, u32)> {
    let fid = m.forma(v);
    let f = m.formas.get(fid);
    if f.classe != Classe::Comum || f.dicionario || m.indices_especiais || m.elementos(v) != Valor::LISTA_VAZIA {
        return None;
    }
    if let Some(&p) = e.planos.get(&fid.0) {
        return Some(p);
    }
    let ini = e.campos.len() as u32;
    let ds: Vec<_> = m.formas.descritores(fid).iter().filter(|d| d.attrs & attr::ENUMERAVEL != 0 && !d.chave.e_simbolo()).copied().collect();
    for d in ds {
        let a = e.citadas.len() as u32;
        let kv = m.chave_para_valor(ChaveProp::Nome(d.chave));
        // Erro (chave enorme) cai no caminho geral, que lança.
        citar_texto(m, &mut e.citadas, kv).ok()?;
        e.campos.push(Campo { chave: d.chave, slot: d.slot, acessor: d.attrs & attr::ACESSOR != 0, a, b: e.citadas.len() as u32 });
    }
    let p = (ini, e.campos.len() as u32);
    e.planos.insert(fid.0, p);
    Some(p)
}

fn objeto(m: &mut Motor, e: &mut Escritor, v: Valor) -> Res<()> {
    let fid = m.forma(v);
    let (ks, faixa) = match &e.lista {
        Some(l) => (l.clone(), None),
        None => match plano(m, e, v) {
            Some(p) => (Vec::new(), Some(p)),
            None => {
                e.sem_tojson.clear();
                (m.chaves_de(v, true, true, false), None)
            }
        },
    };
    e.saida.push(0x7b);
    let mut algum = false;
    let n = faixa.map_or(ks.len(), |(a, b)| (b - a) as usize);
    for i in 0..n {
        let campo = faixa.map(|(a, _)| e.campos[a as usize + i]);
        let k = match campo {
            Some(c) => ChaveProp::Nome(c.chave),
            None => ks[i],
        };
        let x = match campo {
            Some(c) if m.forma(v) == fid && !c.acessor => m.ler_campo(v, c.slot),
            _ => {
                e.sem_tojson.clear();
                m.obter(v, k)?
            }
        };
        let marca = e.saida.len();
        if algum {
            e.saida.push(0x2c);
        }
        quebra(e);
        match k {
            ChaveProp::Nome(_) if campo.is_some() => {
                let c = campo.unwrap();
                e.saida.extend_from_slice(&e.citadas[c.a as usize..c.b as usize]);
            }
            ChaveProp::Nome(c) => {
                let (a, b) = match e.nomes.get(&c) {
                    Some(&f) => f,
                    None => {
                        let a = e.citadas.len() as u32;
                        let kv = m.chave_para_valor(k);
                        citar_texto(m, &mut e.citadas, kv)?;
                        e.nomes.insert(c, (a, e.citadas.len() as u32));
                        (a, e.citadas.len() as u32)
                    }
                };
                e.saida.extend_from_slice(&e.citadas[a as usize..b as usize]);
            }
            _ => {
                let kv = m.chave_para_valor(k);
                citar_texto(m, &mut e.saida, kv)?;
            }
        }
        e.saida.push(0x3a);
        if !e.espaco.is_empty() {
            e.saida.push(0x20);
        }
        if folha(m, e, x) || serializar(m, e, v, k, x)? {
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
        // length enorme (Proxy com 2^33): para no maior texto, não na RAM.
        if e.saida.len() > TEXTO_MAX as usize {
            return m.erro_alcance("Invalid string length");
        }
        quebra(e);
        let x = match m.e_array(v).then(|| m.elemento(v, i as u32)).flatten() {
            Some(x) => x,
            None => {
                e.sem_tojson.clear();
                m.obter_indice(v, i)?
            }
        };
        if !folha(m, e, x) && !serializar(m, e, v, ChaveProp::Indice(i as u32), x)? {
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
    let mut e = Escritor { saida: Vec::new(), pilha: Vec::new(), recuo: Vec::new(), espaco: Vec::new(), substituto: Valor::UNDEFINED, lista: None, nomes: Default::default(), citadas: Vec::new(), sem_tojson: Vec::new(), planos: Default::default(), campos: Vec::new() };
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
        m.texto_u16_res(&e.saida)
    } else {
        Ok(Valor::UNDEFINED)
    }
}
