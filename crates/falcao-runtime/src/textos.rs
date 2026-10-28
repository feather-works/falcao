//! Textos do JS na gaiola: 1 byte por caractere quando cabe em Latin-1, 2 bytes (UTF-16)
//! quando não cabe, e cordas para concatenação preguiçosa (`a + b` não copia nada; o texto
//! só é montado quando alguém lê os caracteres).
//!
//! ```text
//!  plano:  [forma][len][hash][nome+1][unidades…]       (dados a partir do byte 16)
//!  corda:  [forma][len][hash][0][esq][dir]             (depois de achatada: esq = plano, dir = "")
//! ```

use crate::formas::{self, Chave};
use crate::funcoes::Res;
use crate::motor::Motor;
use crate::valor::Valor;
use std::cmp::Ordering;

/// Maior texto aceito (como nos navegadores, perto de 2^30 unidades).
pub const TEXTO_MAX: u32 = (1 << 30) - 25;

/// Abaixo disto, concatenar copia em vez de criar corda.
const CORDA_MIN: u32 = 13;

/// Unidades de um texto plano.
#[derive(Clone, Copy, Debug)]
pub enum Unidades<'a> {
    Latin1(&'a [u8]),
    Utf16(&'a [u16]),
}

impl Unidades<'_> {
    pub fn len(&self) -> usize {
        match self {
            Unidades::Latin1(b) => b.len(),
            Unidades::Utf16(u) => u.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn em(&self, i: usize) -> u16 {
        match self {
            Unidades::Latin1(b) => u16::from(b[i]),
            Unidades::Utf16(u) => u[i],
        }
    }

    pub fn para_u16(&self) -> Vec<u16> {
        match self {
            Unidades::Latin1(b) => b.iter().map(|&c| u16::from(c)).collect(),
            Unidades::Utf16(u) => u.to_vec(),
        }
    }
}

/// Chave de propriedade vinda de um texto: índice de array ou nome.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChaveProp {
    Indice(u32),
    Nome(Chave),
}

/// `"0"`..`"4294967294"` sem zeros à esquerda é índice de array (§6.1.7).
pub fn indice_de_array(u: &[u16]) -> Option<u32> {
    if u.is_empty() || u.len() > 10 || (u.len() > 1 && u[0] == b'0' as u16) {
        return None;
    }
    let mut n: u64 = 0;
    for &c in u {
        if !(b'0' as u16..=b'9' as u16).contains(&c) {
            return None;
        }
        n = n * 10 + u64::from(c - b'0' as u16);
    }
    (n < u64::from(u32::MAX)).then_some(n as u32)
}

impl Motor {
    // ---------------------------------------------------------------- criar

    pub fn texto_latin1(&mut self, b: &[u8]) -> Valor {
        match b.len() {
            0 => return Valor::VAZIA,
            1 => return self.caracteres[b[0] as usize],
            _ => {}
        }
        let n = b.len() as u32;
        assert!(n <= TEXTO_MAX, "texto grande demais");
        let d = self.g.alocar(16 + n);
        self.g.por_u32(d, formas::LATIN1.0);
        self.g.por_u32(d + 4, n);
        self.g.bytes_mut(d + 16, n).copy_from_slice(b);
        Valor::de_deslocamento(d)
    }

    pub fn texto_u16(&mut self, u: &[u16]) -> Valor {
        // Grande: estreita e confere numa passada só; se aparecer unidade larga, a área Latin1
        // fica de lixo (o coletor leva) e o texto sai em UTF-16.
        if u.len() >= 4096 && u.len() as u32 <= TEXTO_MAX {
            let n = u.len() as u32;
            let d = self.g.alocar(16 + n);
            // Cabeçalho válido antes de tudo: mesmo descartada, a área é um objeto que o coletor
            // percorre.
            self.g.por_u32(d, formas::LATIN1.0);
            self.g.por_u32(d + 4, n);
            let mut alto = 0u16;
            for (x, &c) in self.g.bytes_mut(d + 16, n).iter_mut().zip(u) {
                *x = c as u8;
                alto |= c;
            }
            if alto < 256 {
                return Valor::de_deslocamento(d);
            }
        }
        if u.iter().all(|&c| c < 256) {
            match u.len() {
                0 => return Valor::VAZIA,
                1 => return self.caracteres[u[0] as usize],
                _ => {}
            }
            // Estreita direto na gaiola, sem `Vec` no meio.
            let n = u.len() as u32;
            assert!(n <= TEXTO_MAX, "texto grande demais");
            let d = self.g.alocar(16 + n);
            self.g.por_u32(d, formas::LATIN1.0);
            self.g.por_u32(d + 4, n);
            for (x, &c) in self.g.bytes_mut(d + 16, n).iter_mut().zip(u) {
                *x = c as u8;
            }
            return Valor::de_deslocamento(d);
        }
        let n = u.len() as u32;
        assert!(n <= TEXTO_MAX, "texto grande demais");
        let d = self.g.alocar(16 + 2 * n);
        self.g.por_u32(d, formas::UTF16.0);
        self.g.por_u32(d + 4, n);
        self.g.u16s_mut(d + 16, n).copy_from_slice(u);
        Valor::de_deslocamento(d)
    }

    pub fn texto(&mut self, s: &str) -> Valor {
        if s.is_ascii() {
            return self.texto_latin1(s.as_bytes());
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        self.texto_u16(&u)
    }

    /// Texto do nome `c` (alocado uma vez por nome).
    pub fn texto_do_nome(&mut self, c: Chave) -> Valor {
        let v = self.nomes.valores[c.0 as usize];
        if v != Valor::UNDEFINED {
            return v;
        }
        let u = self.nomes.texto(c).to_vec();
        let v = self.texto_u16(&u);
        // Lembra no próprio texto que ele já é o nome `c` (se não for um fixo compartilhado). Um
        // índice ("10") não: como chave ele é `Indice`, e o cache faria `chave_de_texto` dar nome.
        if v != Valor::VAZIA && !(u.len() == 1 && u[0] < 256) && indice_de_array(&u).is_none() {
            self.g.por_u32(v.deslocamento() + 12, c.0 + 1);
        }
        self.nomes.valores[c.0 as usize] = v;
        v
    }

    // ---------------------------------------------------------------- ler

    #[inline]
    pub fn texto_len(&self, v: Valor) -> u32 {
        self.g.u32_em(v.deslocamento() + 4)
    }

    /// Versão plana do texto (achata a corda, se for).
    pub fn achatar(&mut self, v: Valor) -> Valor {
        self.achatar_em(v, false).expect("achatar sem reserva não falha")
    }

    /// [`Motor::achatar`] que dá RangeError se a gaiola não conseguir a memória do texto plano.
    pub(crate) fn achatar_res(&mut self, v: Valor) -> Res<Valor> {
        match self.achatar_em(v, true) {
            Some(p) => Ok(p),
            None => self.erro_alcance("Out of memory"),
        }
    }

    fn achatar_em(&mut self, v: Valor, falivel: bool) -> Option<Valor> {
        if self.forma(v) != formas::CORDA {
            return Some(v);
        }
        let d = v.deslocamento();
        if self.g.valor_em(d + 20) == Valor::VAZIA {
            return Some(self.g.valor_em(d + 16));
        }
        let n = self.texto_len(v);
        // Folhas da esquerda para a direita, sem recursão (cordas de `s += x` são fundas).
        let mut latin1 = true;
        let mut folhas = Vec::new();
        let mut pilha = vec![v];
        while let Some(x) = pilha.pop() {
            let dx = x.deslocamento();
            match formas::FormaId(self.g.u32_em(dx)) {
                formas::CORDA if self.g.valor_em(dx + 20) == Valor::VAZIA => pilha.push(self.g.valor_em(dx + 16)),
                formas::CORDA => {
                    pilha.push(self.g.valor_em(dx + 20));
                    pilha.push(self.g.valor_em(dx + 16));
                }
                f => {
                    latin1 &= f == formas::LATIN1;
                    folhas.push(x);
                }
            }
        }
        // Monta direto na gaiola, folha a folha (sem `Vec` do tamanho do texto no meio).
        let bytes = if latin1 { n } else { 2 * n };
        if falivel && n >= 1 << 20 && !self.g.reservar(16 + u64::from(bytes)) {
            return None;
        }
        let dp = self.g.alocar(16 + bytes);
        self.g.por_u32(dp, if latin1 { formas::LATIN1.0 } else { formas::UTF16.0 });
        self.g.por_u32(dp + 4, n);
        let mut em = dp + 16;
        for f in folhas {
            let (df, nf) = (f.deslocamento(), self.texto_len(f));
            if latin1 || self.g.u32_em(df) == formas::UTF16.0 {
                let b = if latin1 { nf } else { 2 * nf };
                self.g.copiar(df + 16, em, b);
                em += b;
            } else {
                let w: Vec<u16> = self.g.bytes(df + 16, nf).iter().map(|&c| u16::from(c)).collect();
                self.g.u16s_mut(em, nf).copy_from_slice(&w);
                em += 2 * nf;
            }
        }
        let plano = Valor::de_deslocamento(dp);
        let d = v.deslocamento();
        self.g.por_valor(d, d + 16, plano);
        self.g.por_valor(d, d + 20, Valor::VAZIA);
        Some(plano)
    }

    /// Unidades de um texto **plano**.
    #[inline]
    pub fn unidades_planas(&self, v: Valor) -> Unidades<'_> {
        let d = v.deslocamento();
        let n = self.g.u32_em(d + 4);
        if self.g.u32_em(d) == formas::LATIN1.0 {
            Unidades::Latin1(self.g.bytes(d + 16, n))
        } else {
            debug_assert_eq!(self.g.u32_em(d), formas::UTF16.0);
            Unidades::Utf16(self.g.u16s(d + 16, n))
        }
    }

    /// Unidades de qualquer texto (achata se precisar).
    pub fn unidades(&mut self, v: Valor) -> Unidades<'_> {
        let p = self.achatar(v);
        self.unidades_planas(p)
    }

    /// [`Motor::texto_u16`] que, com texto grande, confere antes se a gaiola consegue a
    /// memória: RangeError em vez de derrubar o processo.
    pub fn texto_u16_res(&mut self, u: &[u16]) -> Res<Valor> {
        if u.len() >= 1 << 20 {
            if u.len() > TEXTO_MAX as usize {
                return self.erro_alcance("Invalid string length");
            }
            // Reserva o bloco Latin-1 que `texto_u16` tenta primeiro (sem varrer o texto duas vezes).
            if !self.g.reservar(16 + u.len() as u64) {
                return self.erro_alcance("Out of memory");
            }
        }
        Ok(self.texto_u16(u))
    }

    /// Texto plano novo de `n` unidades (zeradas, `n` >= 2) para a nativa preencher no lugar,
    /// sem `Vec` no meio; RangeError se a gaiola não conseguir a memória.
    pub(crate) fn texto_plano_novo(&mut self, n: u32, latin1: bool) -> Res<Valor> {
        let bytes = if latin1 { n } else { 2 * n };
        if n > TEXTO_MAX || !self.g.reservar(16 + u64::from(bytes)) {
            return self.erro_alcance(if n > TEXTO_MAX { "Invalid string length" } else { "Out of memory" });
        }
        let d = self.g.alocar(16 + bytes);
        self.g.por_u32(d, if latin1 { formas::LATIN1.0 } else { formas::UTF16.0 });
        self.g.por_u32(d + 4, n);
        Ok(Valor::de_deslocamento(d))
    }

    pub fn texto_u16_de(&mut self, v: Valor) -> Vec<u16> {
        self.unidades(v).para_u16()
    }

    /// Texto como `String` do Rust (surrogate solto vira U+FFFD).
    pub fn texto_rust(&mut self, v: Valor) -> String {
        match self.unidades(v) {
            Unidades::Latin1(b) => b.iter().map(|&c| c as char).collect(),
            Unidades::Utf16(u) => String::from_utf16_lossy(u),
        }
    }

    pub fn unidade_em(&mut self, v: Valor, i: u32) -> u16 {
        self.unidades(v).em(i as usize)
    }

    // ---------------------------------------------------------------- operações

    pub fn concatenar(&mut self, a: Valor, b: Valor) -> Res<Valor> {
        let (la, lb) = (self.texto_len(a), self.texto_len(b));
        if la == 0 {
            return Ok(b);
        }
        if lb == 0 {
            return Ok(a);
        }
        let n = la + lb;
        if n > TEXTO_MAX {
            return self.erro_alcance("Invalid string length");
        }
        if n < CORDA_MIN {
            // Os dois em Latin-1 (o comum: `'k' + i`): os bytes juntos direto, sem passar por
            // UTF-16 nem pelo `malloc`.
            let mut buf = [0u8; CORDA_MIN as usize];
            let mut k = 0;
            for v in [a, b] {
                match self.unidades(v) {
                    Unidades::Latin1(x) => {
                        buf[k..k + x.len()].copy_from_slice(x);
                        k += x.len();
                    }
                    Unidades::Utf16(_) => break,
                }
            }
            if k == n as usize {
                return Ok(self.texto_latin1(&buf[..k]));
            }
            let mut u = self.texto_u16_de(a);
            u.extend(self.texto_u16_de(b));
            return Ok(self.texto_u16(&u));
        }
        let d = self.g.alocar(24);
        self.g.por_u32(d, formas::CORDA.0);
        self.g.por_u32(d + 4, n);
        self.g.por_valor(d, d + 16, a);
        self.g.por_valor(d, d + 20, b);
        Ok(Valor::de_deslocamento(d))
    }

    /// `texto.substring(ini, fim)` (índices já validados, `ini <= fim <= len`).
    pub fn subtexto(&mut self, v: Valor, ini: u32, fim: u32) -> Valor {
        if ini == 0 && fim == self.texto_len(v) {
            return v;
        }
        match self.unidades(v) {
            Unidades::Latin1(b) => {
                let parte = b[ini as usize..fim as usize].to_vec();
                self.texto_latin1(&parte)
            }
            Unidades::Utf16(u) => {
                let parte = u[ini as usize..fim as usize].to_vec();
                self.texto_u16(&parte)
            }
        }
    }

    pub fn textos_iguais(&mut self, a: Valor, b: Valor) -> bool {
        if a == b {
            return true;
        }
        if self.texto_len(a) != self.texto_len(b) {
            return false;
        }
        // Os dois já internados como nomes diferentes: diferentes.
        let (na, nb) = (self.nome_cacheado(a), self.nome_cacheado(b));
        if na != 0 && nb != 0 {
            return na == nb;
        }
        let pa = self.achatar(a);
        let pb = self.achatar(b);
        match (self.unidades_planas(pa), self.unidades_planas(pb)) {
            (Unidades::Latin1(x), Unidades::Latin1(y)) => x == y,
            (Unidades::Utf16(x), Unidades::Utf16(y)) => x == y,
            // Texto plano é Latin-1 sempre que cabe, então misto só é igual se o UTF-16 couber.
            (x, y) => (0..x.len()).all(|i| x.em(i) == y.em(i)),
        }
    }

    /// Ordem por unidades UTF-16 (o `<` de textos).
    pub fn comparar_textos(&mut self, a: Valor, b: Valor) -> Ordering {
        let pa = self.achatar(a);
        let pb = self.achatar(b);
        let (x, y) = (self.unidades_planas(pa), self.unidades_planas(pb));
        if let (Unidades::Latin1(x), Unidades::Latin1(y)) = (x, y) {
            return x.cmp(y);
        }
        let n = x.len().min(y.len());
        for i in 0..n {
            match x.em(i).cmp(&y.em(i)) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        x.len().cmp(&y.len())
    }

    #[inline]
    fn nome_cacheado(&self, v: Valor) -> u32 {
        if self.forma(v) == formas::CORDA {
            return 0;
        }
        self.g.u32_em(v.deslocamento() + 12)
    }

    /// Chave de propriedade do texto: índice de array ou nome internado (e lembra o nome no
    /// próprio texto, para a próxima vez ser só uma leitura).
    pub fn chave_de_texto(&mut self, v: Valor) -> ChaveProp {
        let p = self.achatar(v);
        let n = self.nome_cacheado(p);
        if n != 0 {
            return ChaveProp::Nome(Chave(n - 1));
        }
        let u = self.unidades_planas(p).para_u16();
        if let Some(i) = indice_de_array(&u) {
            return ChaveProp::Indice(i);
        }
        let c = self.nomes.internar(&u);
        let fixo = p == Valor::VAZIA || (u.len() == 1 && u[0] < 256);
        if !fixo {
            self.g.por_u32(p.deslocamento() + 12, c.0 + 1);
        }
        ChaveProp::Nome(c)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn cria_concatena_achata_compara() {
        let mut m = Motor::novo();
        let a = m.texto("olá, ");
        let b = m.texto("mundo grande e bonito");
        let c = m.concatenar(a, b).unwrap();
        assert_eq!(m.forma(c), formas::CORDA);
        assert_eq!(m.texto_len(c), 26);
        assert_eq!(m.texto_rust(c), "olá, mundo grande e bonito");
        // Corda funda (s += x mil vezes) achata sem recursão.
        let mut s = Valor::VAZIA;
        for _ in 0..100_000 {
            let x = m.texto("ab");
            s = m.concatenar(s, x).unwrap();
        }
        assert_eq!(m.texto_len(s), 200_000);
        assert_eq!(m.unidade_em(s, 199_999), b'b' as u16);
        let e = m.texto("€uro");
        assert_eq!(m.forma(e), formas::UTF16);
        let e2 = m.texto("€uro");
        assert!(m.textos_iguais(e, e2));
        assert_eq!(m.comparar_textos(a, b), Ordering::Greater);
        let sub = m.subtexto(c, 5, 10);
        assert_eq!(m.texto_rust(sub), "mundo");
        let n1 = m.texto("comprimento");
        let ChaveProp::Nome(k) = m.chave_de_texto(n1) else { panic!() };
        let n2 = m.texto("comprimento");
        assert_eq!(m.chave_de_texto(n2), ChaveProp::Nome(k));
        let l = m.texto("length");
        assert_eq!(m.chave_de_texto(l), ChaveProp::Nome(crate::nomes::fixos::LENGTH));
        let i = m.texto("42");
        assert_eq!(m.chave_de_texto(i), ChaveProp::Indice(42));
        let z = m.texto("042");
        assert!(matches!(m.chave_de_texto(z), ChaveProp::Nome(_)));
        assert_eq!(m.texto_do_nome(k), m.texto_do_nome(k));
    }
}
