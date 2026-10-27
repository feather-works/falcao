//! Léxico (ECMA-262 seção 12): texto → símbolos, um de cada vez, a pedido da sintaxe.
//!
//! - Quem diz se `/` é divisão ou expressão regular é a sintaxe ([`Objetivo`]); quem diz que
//!   um `}` continua um template também ([`Lexico::continuar_template`]).
//! - Nomes saem como [`Atomo`]; as palavras do JS são átomos fixos (`palavras::*`).
//! - Textos sem escape não copiam nada: o símbolo guarda só a faixa no código-fonte. Os
//!   com escape são decodificados uma vez, em UTF-16, numa tabela do léxico.

use crate::atomos::{Atomo, Atomos};
use crate::numeros;
use crate::unicode::{espaco, fim_de_linha, inicio_de_nome, parte_de_nome};

/// Pontuadores (ECMA-262 §12.8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pont {
    AbreChave, FechaChave, AbreParen, FechaParen, AbreColchete, FechaColchete,
    Ponto, Reticencias, PontoVirgula, Virgula, Interrogacao, InterrogacaoPonto, DoisPontos, Seta,
    Menor, Maior, MenorIgual, MaiorIgual, Igual2, Diferente, Igual3, Diferente2,
    Mais, Menos, Vezes, Resto, Potencia, Barra, MaisMais, MenosMenos,
    DeslEsq, DeslDir, DeslDirSemSinal, E, Ou, OuExclusivo, Nao, Til, EE, OuOu, Coalesce,
    Atrib, MaisAtrib, MenosAtrib, VezesAtrib, BarraAtrib, RestoAtrib, PotenciaAtrib,
    DeslEsqAtrib, DeslDirAtrib, DeslDirSemSinalAtrib, EAtrib, OuAtrib, OuExclusivoAtrib,
    EEAtrib, OuOuAtrib, CoalesceAtrib,
}

/// O que o símbolo é.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tipo {
    /// Nome ou palavra (`dado` = [`Dado::Atomo`]).
    Nome,
    /// `#nome` de campo privado (`dado` = [`Dado::Atomo`], sem o `#`).
    NomePrivado,
    /// `dado` = [`Dado::Numero`].
    Numero,
    /// `123n` (`dado` = [`Dado::Texto`] com os dígitos decimais, sem `_`).
    BigInt,
    /// `"..."` ou `'...'` (`dado` = [`Dado::Faixa`] sem escapes, ou [`Dado::Texto`]).
    Texto,
    /// Pedaço de template (`dado` = [`Dado::Template`]).
    Template(Template),
    /// `/corpo/flags` (`dado` = [`Dado::Regex`]).
    Regex,
    Pont(Pont),
    Fim,
}

/// Onde o pedaço de template fica: `` `a${ `` (cabeça), `` }b${ `` (meio), `` }c` `` (cauda),
/// `` `d` `` (inteiro, sem substituição).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Template {
    pub cabeca: bool,
    pub cauda: bool,
}

/// O valor que acompanha o símbolo.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dado {
    Nada,
    Atomo(Atomo),
    Numero(f64),
    /// Texto sem escapes: a faixa (em bytes) no código-fonte, sem as aspas.
    Faixa(u32, u32),
    /// Índice em [`Lexico::texto`] (UTF-16).
    Texto(u32),
    /// Índice em [`Lexico::template`].
    Template(u32),
    /// Faixas do corpo e das flags no código-fonte.
    Regex { corpo: (u32, u32), flags: (u32, u32) },
}

/// Um símbolo. `inicio..fim` em bytes no código-fonte.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Simbolo {
    pub tipo: Tipo,
    pub inicio: u32,
    pub fim: u32,
    /// Houve fim de linha entre o símbolo anterior e este (inserção automática de `;`).
    pub quebra_antes: bool,
    /// Nome com `\u` (não vale como palavra reservada), número com octal legado (`017`,
    /// `08`) ou texto com escape octal (`"\1"`): proibidos no modo estrito.
    pub escapado: bool,
    pub dado: Dado,
}

/// Pedaço de template: o valor "cozido" (`None` se tem escape inválido, permitido só em
/// template marcado) e o "cru" (como está no fonte, com CR/CRLF virando LF).
#[derive(Clone, Debug, PartialEq)]
pub struct ParteTemplate {
    pub cozido: Option<Vec<u16>>,
    pub cru: Vec<u16>,
}

/// O que a sintaxe espera neste ponto.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Objetivo {
    /// Depois de uma expressão: `/` é divisão.
    Divisao,
    /// No começo de uma expressão: `/` começa uma expressão regular.
    Regex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErroLexico {
    pub posicao: u32,
    pub mensagem: &'static str,
}

type R<T> = Result<T, ErroLexico>;

pub struct Lexico<'a> {
    fonte: &'a str,
    b: &'a [u8],
    pos: usize,
    /// Módulo: sem comentários `<!--`/`-->` (Anexo B só vale em script).
    modulo: bool,
    pub atomos: Atomos,
    textos: Vec<Vec<u16>>,
    templates: Vec<ParteTemplate>,
    /// Buffer reaproveitado para nomes com escape.
    nome: String,
}

impl<'a> Lexico<'a> {
    pub fn novo(fonte: &'a str, modulo: bool) -> Self {
        Self::com_atomos(fonte, modulo, Atomos::novo())
    }

    /// Com uma tabela de átomos já existente (a do contexto, compartilhada entre scripts).
    pub fn com_atomos(fonte: &'a str, modulo: bool, atomos: Atomos) -> Self {
        let mut l = Self { fonte, b: fonte.as_bytes(), pos: 0, modulo, atomos, textos: Vec::new(), templates: Vec::new(), nome: String::new() };
        // Hashbang (§12.5): só na primeira linha.
        if l.b.starts_with(b"#!") {
            while l.pos < l.b.len() && !l.fim_de_linha_aqui() {
                l.avancar_char();
            }
        }
        l
    }

    pub fn fonte(&self) -> &'a str {
        self.fonte
    }

    /// Texto decodificado de um [`Dado::Texto`].
    pub fn texto(&self, i: u32) -> &[u16] {
        &self.textos[i as usize]
    }

    pub fn template(&self, i: u32) -> &ParteTemplate {
        &self.templates[i as usize]
    }

    /// Posição atual (em bytes), para a sintaxe voltar atrás se precisar.
    pub fn posicao(&self) -> u32 {
        self.pos as u32
    }

    /// Volta (ou avança) o léxico para `pos`, que deve ser o fim de um símbolo já lido.
    pub fn ir_para(&mut self, pos: u32) {
        self.pos = pos as usize;
    }

    fn erro<T>(&self, posicao: usize, mensagem: &'static str) -> R<T> {
        Err(ErroLexico { posicao: posicao as u32, mensagem })
    }

    #[inline]
    fn olhar(&self, d: usize) -> u8 {
        *self.b.get(self.pos + d).unwrap_or(&0)
    }

    #[inline]
    fn char_aqui(&self) -> Option<char> {
        self.fonte[self.pos..].chars().next()
    }

    #[inline]
    fn avancar_char(&mut self) {
        let c = self.b[self.pos];
        self.pos += if c < 0x80 { 1 } else if c < 0xE0 { 2 } else if c < 0xF0 { 3 } else { 4 };
    }

    fn fim_de_linha_aqui(&self) -> bool {
        match self.b[self.pos] {
            b'\n' | b'\r' => true,
            0xE2 => self.olhar(1) == 0x80 && matches!(self.olhar(2), 0xA8 | 0xA9),
            _ => false,
        }
    }

    /// Pula espaços e comentários; devolve se passou por algum fim de linha.
    fn pular(&mut self) -> R<bool> {
        let mut quebra = false;
        // Início de linha conta para o `-->` do Anexo B.
        let mut inicio_linha = self.pos == 0;
        while self.pos < self.b.len() {
            let c = self.b[self.pos];
            match c {
                b' ' | b'\t' | 0x0B | 0x0C => self.pos += 1,
                b'\n' | b'\r' => {
                    self.pos += 1;
                    quebra = true;
                    inicio_linha = true;
                }
                b'/' if self.olhar(1) == b'/' => self.pular_linha(),
                b'/' if self.olhar(1) == b'*' => {
                    let ini = self.pos;
                    self.pos += 2;
                    loop {
                        if self.pos >= self.b.len() {
                            return self.erro(ini, "comentário /* sem fechar");
                        }
                        if self.b[self.pos] == b'*' && self.olhar(1) == b'/' {
                            self.pos += 2;
                            break;
                        }
                        if self.fim_de_linha_aqui() {
                            quebra = true;
                            inicio_linha = true;
                        }
                        self.avancar_char();
                    }
                }
                b'<' if !self.modulo && self.b[self.pos..].starts_with(b"<!--") => self.pular_linha(),
                b'-' if !self.modulo && (quebra || inicio_linha) && self.b[self.pos..].starts_with(b"-->") => self.pular_linha(),
                c if c < 0x80 => break,
                _ => {
                    let ch = self.char_aqui().unwrap_or('\0');
                    if fim_de_linha(ch) {
                        quebra = true;
                        inicio_linha = true;
                    } else if !espaco(ch) {
                        break;
                    }
                    self.avancar_char();
                }
            }
        }
        Ok(quebra)
    }

    fn pular_linha(&mut self) {
        while self.pos < self.b.len() && !self.fim_de_linha_aqui() {
            self.avancar_char();
        }
    }

    /// Próximo símbolo.
    pub fn proximo(&mut self, objetivo: Objetivo) -> R<Simbolo> {
        let quebra_antes = self.pular()?;
        let inicio = self.pos;
        let mut s = Simbolo { tipo: Tipo::Fim, inicio: inicio as u32, fim: inicio as u32, quebra_antes, escapado: false, dado: Dado::Nada };
        if self.pos >= self.b.len() {
            return Ok(s);
        }
        let c = self.b[self.pos];
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'$' | b'_' | b'\\' => self.ler_nome(&mut s)?,
            b'0'..=b'9' => self.ler_numero(&mut s)?,
            b'.' if self.olhar(1).is_ascii_digit() => self.ler_numero(&mut s)?,
            b'"' | b'\'' => self.ler_texto(&mut s, c)?,
            b'`' => {
                self.pos += 1;
                self.ler_template(&mut s, true)?;
            }
            b'#' => {
                self.pos += 1;
                let c2 = self.char_aqui().unwrap_or('\0');
                if !(inicio_de_nome(c2) || c2 == '\\') {
                    return self.erro(inicio, "# sem nome de campo privado");
                }
                self.ler_nome(&mut s)?;
                s.tipo = Tipo::NomePrivado;
            }
            b'/' if objetivo == Objetivo::Regex => self.ler_regex(&mut s)?,
            c if c < 0x80 => s.tipo = Tipo::Pont(self.ler_pont()?),
            _ => {
                let ch = self.char_aqui().unwrap_or('\0');
                if !inicio_de_nome(ch) {
                    return self.erro(inicio, "caractere inválido");
                }
                self.ler_nome(&mut s)?;
            }
        }
        s.fim = self.pos as u32;
        Ok(s)
    }

    /// Depois do `}` que fecha um `${...}`: lê o resto do template (meio ou cauda). O `}`
    /// já foi devolvido como `Pont::FechaChave`, e o léxico está logo depois dele.
    pub fn continuar_template(&mut self, chave: &Simbolo) -> R<Simbolo> {
        let mut s = Simbolo { tipo: Tipo::Fim, inicio: chave.inicio, fim: chave.fim, quebra_antes: chave.quebra_antes, escapado: false, dado: Dado::Nada };
        self.pos = chave.fim as usize;
        self.ler_template(&mut s, false)?;
        s.fim = self.pos as u32;
        Ok(s)
    }

    /// A sintaxe leu `/` ou `/=` como pontuador, mas ali começa uma expressão regular.
    pub fn reler_como_regex(&mut self, barra: &Simbolo) -> R<Simbolo> {
        let mut s = *barra;
        self.pos = barra.inicio as usize;
        self.ler_regex(&mut s)?;
        s.fim = self.pos as u32;
        Ok(s)
    }

    fn ler_pont(&mut self) -> R<Pont> {
        use Pont::*;
        let c = self.b[self.pos];
        let (p, n) = match (c, self.olhar(1), self.olhar(2), self.olhar(3)) {
            (b'{', ..) => (AbreChave, 1),
            (b'}', ..) => (FechaChave, 1),
            (b'(', ..) => (AbreParen, 1),
            (b')', ..) => (FechaParen, 1),
            (b'[', ..) => (AbreColchete, 1),
            (b']', ..) => (FechaColchete, 1),
            (b';', ..) => (PontoVirgula, 1),
            (b',', ..) => (Virgula, 1),
            (b':', ..) => (DoisPontos, 1),
            (b'~', ..) => (Til, 1),
            (b'.', b'.', b'.', _) => (Reticencias, 3),
            (b'.', ..) => (Ponto, 1),
            (b'?', b'?', b'=', _) => (CoalesceAtrib, 3),
            (b'?', b'?', ..) => (Coalesce, 2),
            // `a?.5:0` é condicional, não `?.`.
            (b'?', b'.', d, _) if !d.is_ascii_digit() => (InterrogacaoPonto, 2),
            (b'?', ..) => (Interrogacao, 1),
            (b'=', b'=', b'=', _) => (Igual3, 3),
            (b'=', b'=', ..) => (Igual2, 2),
            (b'=', b'>', ..) => (Seta, 2),
            (b'=', ..) => (Atrib, 1),
            (b'!', b'=', b'=', _) => (Diferente2, 3),
            (b'!', b'=', ..) => (Diferente, 2),
            (b'!', ..) => (Nao, 1),
            (b'<', b'<', b'=', _) => (DeslEsqAtrib, 3),
            (b'<', b'<', ..) => (DeslEsq, 2),
            (b'<', b'=', ..) => (MenorIgual, 2),
            (b'<', ..) => (Menor, 1),
            (b'>', b'>', b'>', b'=') => (DeslDirSemSinalAtrib, 4),
            (b'>', b'>', b'>', _) => (DeslDirSemSinal, 3),
            (b'>', b'>', b'=', _) => (DeslDirAtrib, 3),
            (b'>', b'>', ..) => (DeslDir, 2),
            (b'>', b'=', ..) => (MaiorIgual, 2),
            (b'>', ..) => (Maior, 1),
            (b'+', b'+', ..) => (MaisMais, 2),
            (b'+', b'=', ..) => (MaisAtrib, 2),
            (b'+', ..) => (Mais, 1),
            (b'-', b'-', ..) => (MenosMenos, 2),
            (b'-', b'=', ..) => (MenosAtrib, 2),
            (b'-', ..) => (Menos, 1),
            (b'*', b'*', b'=', _) => (PotenciaAtrib, 3),
            (b'*', b'*', ..) => (Potencia, 2),
            (b'*', b'=', ..) => (VezesAtrib, 2),
            (b'*', ..) => (Vezes, 1),
            (b'%', b'=', ..) => (RestoAtrib, 2),
            (b'%', ..) => (Resto, 1),
            (b'/', b'=', ..) => (BarraAtrib, 2),
            (b'/', ..) => (Barra, 1),
            (b'&', b'&', b'=', _) => (EEAtrib, 3),
            (b'&', b'&', ..) => (EE, 2),
            (b'&', b'=', ..) => (EAtrib, 2),
            (b'&', ..) => (E, 1),
            (b'|', b'|', b'=', _) => (OuOuAtrib, 3),
            (b'|', b'|', ..) => (OuOu, 2),
            (b'|', b'=', ..) => (OuAtrib, 2),
            (b'|', ..) => (Ou, 1),
            (b'^', b'=', ..) => (OuExclusivoAtrib, 2),
            (b'^', ..) => (OuExclusivo, 1),
            _ => return self.erro(self.pos, "caractere inválido"),
        };
        self.pos += n;
        Ok(p)
    }

    /// Nome ou palavra, com `\uXXXX`/`\u{...}` permitidos.
    fn ler_nome(&mut self, s: &mut Simbolo) -> R<()> {
        let ini = self.pos;
        // Caminho rápido: ASCII sem escape, interna direto a faixa do fonte.
        while self.pos < self.b.len() {
            let c = self.b[self.pos];
            if c.is_ascii_alphanumeric() || c == b'$' || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let rapido = self.pos >= self.b.len() || (self.b[self.pos] < 0x80 && self.b[self.pos] != b'\\');
        if rapido && self.pos > ini {
            s.tipo = Tipo::Nome;
            s.dado = Dado::Atomo(self.atomos.internar(&self.fonte[ini..self.pos]));
            return Ok(());
        }
        // Caminho geral: Unicode e escapes.
        self.nome.clear();
        self.nome.push_str(&self.fonte[ini..self.pos]);
        let mut primeiro = self.pos == ini;
        while self.pos < self.b.len() {
            let (ch, escape) = if self.b[self.pos] == b'\\' {
                let p = self.pos;
                if self.olhar(1) != b'u' {
                    return self.erro(p, "\\ fora de \\u num nome");
                }
                self.pos += 2;
                let v = self.ler_escape_u()?;
                (char::from_u32(v).unwrap_or('\0'), true)
            } else {
                let Some(ch) = self.char_aqui() else { break };
                (ch, false)
            };
            let vale = if primeiro { inicio_de_nome(ch) } else { parte_de_nome(ch) };
            if !vale {
                if escape {
                    return self.erro(self.pos, "escape \\u com caractere que não pode estar num nome");
                }
                break;
            }
            if escape {
                s.escapado = true;
            } else {
                self.avancar_char();
            }
            self.nome.push(ch);
            primeiro = false;
        }
        s.tipo = Tipo::Nome;
        let nome = std::mem::take(&mut self.nome);
        s.dado = Dado::Atomo(self.atomos.internar(&nome));
        self.nome = nome;
        Ok(())
    }

    /// Depois de `\u`: `XXXX` ou `{X...}` (até 10FFFF).
    fn ler_escape_u(&mut self) -> R<u32> {
        let p = self.pos;
        if self.olhar(0) == b'{' {
            self.pos += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            while let Some(d) = (self.olhar(0) as char).to_digit(16) {
                v = v.saturating_mul(16).saturating_add(d);
                self.pos += 1;
                n += 1;
            }
            if n == 0 || self.olhar(0) != b'}' || v > 0x10FFFF {
                return self.erro(p, "escape \\u{...} inválido");
            }
            self.pos += 1;
            Ok(v)
        } else {
            let mut v = 0;
            for _ in 0..4 {
                let Some(d) = (self.olhar(0) as char).to_digit(16) else {
                    return self.erro(p, "escape \\u inválido");
                };
                v = v * 16 + d;
                self.pos += 1;
            }
            Ok(v)
        }
    }

    /// Dígitos em `base`, com `_` entre dígitos; devolve o texto sem `_`.
    fn digitos(&mut self, base: u32, saida: &mut String) -> R<()> {
        let mut anterior_digito = false;
        loop {
            let c = self.olhar(0);
            if c == b'_' {
                if !anterior_digito || !(self.olhar(1) as char).is_digit(base) {
                    return self.erro(self.pos, "separador _ fora do lugar num número");
                }
                self.pos += 1;
                anterior_digito = false;
            } else if (c as char).is_digit(base) {
                saida.push(c as char);
                self.pos += 1;
                anterior_digito = true;
            } else {
                return Ok(());
            }
        }
    }

    fn ler_numero(&mut self, s: &mut Simbolo) -> R<()> {
        let ini = self.pos;
        s.tipo = Tipo::Numero;
        let mut texto = String::new();
        let c0 = self.olhar(0);
        let c1 = self.olhar(1) | 0x20;
        if c0 == b'0' && matches!(c1, b'x' | b'o' | b'b') {
            let base = match c1 {
                b'x' => 16,
                b'o' => 8,
                _ => 2,
            };
            self.pos += 2;
            self.digitos(base, &mut texto)?;
            if texto.is_empty() {
                return self.erro(ini, "número sem dígitos depois do prefixo");
            }
            if self.olhar(0) == b'n' {
                self.pos += 1;
                self.fim_de_numero(ini)?;
                return self.bigint(s, &texto, base);
            }
            s.dado = Dado::Numero(numeros::inteiro(&texto, base));
            return self.fim_de_numero(ini);
        }
        // Octal legado (`017`) e decimal com zero à esquerda (`08`): só fora do modo estrito.
        if c0 == b'0' && self.olhar(1).is_ascii_digit() {
            s.escapado = true;
            while self.olhar(0).is_ascii_digit() {
                texto.push(self.olhar(0) as char);
                self.pos += 1;
            }
            if texto.bytes().all(|d| d < b'8') {
                s.dado = Dado::Numero(numeros::inteiro(&texto, 8));
                return self.fim_de_numero(ini);
            }
            // `08`, `09.5`: decimal (pode ter fração e expoente).
        } else {
            self.digitos(10, &mut texto)?;
        }
        if self.olhar(0) == b'n' && !texto.is_empty() && !s.escapado {
            self.pos += 1;
            self.fim_de_numero(ini)?;
            return self.bigint(s, &texto, 10);
        }
        if self.olhar(0) == b'.' {
            self.pos += 1;
            texto.push('.');
            if self.olhar(0) == b'_' {
                return self.erro(self.pos, "separador _ fora do lugar num número");
            }
            self.digitos(10, &mut texto)?;
        }
        if self.olhar(0) | 0x20 == b'e' {
            let p = self.pos;
            self.pos += 1;
            texto.push('e');
            if matches!(self.olhar(0), b'+' | b'-') {
                texto.push(self.olhar(0) as char);
                self.pos += 1;
            }
            let antes = texto.len();
            self.digitos(10, &mut texto)?;
            if texto.len() == antes {
                return self.erro(p, "expoente sem dígitos");
            }
        }
        s.dado = Dado::Numero(numeros::decimal(&texto));
        self.fim_de_numero(ini)
    }

    /// Número não pode encostar em nome ou dígito (`3in`, `1.5.3` já é outro símbolo).
    fn fim_de_numero(&mut self, ini: usize) -> R<()> {
        if self.pos < self.b.len() {
            let c = self.char_aqui().unwrap_or('\0');
            if inicio_de_nome(c) || c.is_ascii_digit() || c == '\\' {
                return self.erro(ini, "número encostado em nome");
            }
        }
        Ok(())
    }

    fn bigint(&mut self, s: &mut Simbolo, digitos: &str, base: u32) -> R<()> {
        s.tipo = Tipo::BigInt;
        let decimal = if base == 10 { digitos.trim_start_matches('0').to_string() } else { bigint_para_decimal(digitos, base) };
        let decimal = if decimal.is_empty() { "0".to_string() } else { decimal };
        self.textos.push(decimal.encode_utf16().collect());
        s.dado = Dado::Texto(self.textos.len() as u32 - 1);
        Ok(())
    }

    fn ler_texto(&mut self, s: &mut Simbolo, aspa: u8) -> R<()> {
        let ini = self.pos;
        self.pos += 1;
        s.tipo = Tipo::Texto;
        // Caminho rápido: sem `\` nem fim de linha, só a faixa.
        let corpo = self.pos;
        while self.pos < self.b.len() {
            let c = self.b[self.pos];
            if c == aspa {
                s.dado = Dado::Faixa(corpo as u32, self.pos as u32);
                self.pos += 1;
                return Ok(());
            }
            if c == b'\\' || c == b'\n' || c == b'\r' {
                break;
            }
            self.pos += 1;
        }
        let mut u: Vec<u16> = self.fonte[corpo..self.pos].encode_utf16().collect();
        loop {
            if self.pos >= self.b.len() {
                return self.erro(ini, "texto sem fechar");
            }
            let c = self.b[self.pos];
            if c == aspa {
                self.pos += 1;
                break;
            }
            match c {
                b'\n' | b'\r' => return self.erro(self.pos, "fim de linha dentro de texto"),
                b'\\' => {
                    self.pos += 1;
                    match self.escape(false)? {
                        Escape::Unidades(a, b) => {
                            u.push(a);
                            if let Some(b) = b {
                                u.push(b);
                            }
                        }
                        Escape::Continuacao => {}
                        Escape::OctalLegado(v) => {
                            s.escapado = true;
                            u.push(v);
                        }
                    }
                }
                _ => {
                    let ch = self.char_aqui().unwrap_or('\0');
                    self.avancar_char();
                    let mut buf = [0u16; 2];
                    u.extend_from_slice(ch.encode_utf16(&mut buf));
                }
            }
        }
        self.textos.push(u);
        s.dado = Dado::Texto(self.textos.len() as u32 - 1);
        Ok(())
    }

    /// Depois da `\`. Em template, octal (`\1`) e `\0` seguido de dígito são inválidos.
    fn escape(&mut self, template: bool) -> R<Escape> {
        let p = self.pos;
        if self.pos >= self.b.len() {
            return self.erro(p, "escape no fim do texto");
        }
        let c = self.b[self.pos];
        let simples = |v: u8| Ok(Escape::Unidades(u16::from(v), None));
        match c {
            b'n' => { self.pos += 1; simples(b'\n') }
            b't' => { self.pos += 1; simples(b'\t') }
            b'r' => { self.pos += 1; simples(b'\r') }
            b'b' => { self.pos += 1; simples(0x08) }
            b'f' => { self.pos += 1; simples(0x0C) }
            b'v' => { self.pos += 1; simples(0x0B) }
            b'0' if !self.olhar(1).is_ascii_digit() => { self.pos += 1; simples(0) }
            b'0'..=b'7' => {
                if template {
                    return self.erro(p, "escape octal em template");
                }
                // `\1`..`\377`: até 3 dígitos octais, valor até 255.
                let mut v = u16::from(c - b'0');
                self.pos += 1;
                let max = if c <= b'3' { 2 } else { 1 };
                for _ in 0..max {
                    let d = self.olhar(0);
                    if !(b'0'..=b'7').contains(&d) {
                        break;
                    }
                    v = v * 8 + u16::from(d - b'0');
                    self.pos += 1;
                }
                Ok(Escape::OctalLegado(v))
            }
            b'8' | b'9' => {
                if template {
                    return self.erro(p, "escape \\8/\\9 em template");
                }
                self.pos += 1;
                Ok(Escape::OctalLegado(u16::from(c)))
            }
            b'x' => {
                self.pos += 1;
                let (Some(a), Some(b)) = ((self.olhar(0) as char).to_digit(16), (self.olhar(1) as char).to_digit(16)) else {
                    return self.erro(p, "escape \\x inválido");
                };
                self.pos += 2;
                Ok(Escape::Unidades((a * 16 + b) as u16, None))
            }
            b'u' => {
                self.pos += 1;
                let v = self.ler_escape_u()?;
                Ok(unidades(v))
            }
            b'\r' => {
                self.pos += 1;
                if self.olhar(0) == b'\n' {
                    self.pos += 1;
                }
                Ok(Escape::Continuacao)
            }
            b'\n' => {
                self.pos += 1;
                Ok(Escape::Continuacao)
            }
            _ => {
                let ch = self.char_aqui().unwrap_or('\0');
                self.avancar_char();
                if ch == '\u{2028}' || ch == '\u{2029}' {
                    return Ok(Escape::Continuacao);
                }
                // Qualquer outro caractere vale ele mesmo (`\'`, `\"`, `\\`, `\a`...).
                Ok(unidades(ch as u32))
            }
        }
    }

    /// Depois de `` ` `` (cabeça) ou de `}` (meio/cauda): até `` ` `` ou `${`.
    fn ler_template(&mut self, s: &mut Simbolo, cabeca: bool) -> R<()> {
        let ini = self.pos;
        let mut cozido: Option<Vec<u16>> = Some(Vec::new());
        let mut cru: Vec<u16> = Vec::new();
        let cauda;
        loop {
            if self.pos >= self.b.len() {
                return self.erro(ini, "template sem fechar");
            }
            let c = self.b[self.pos];
            match c {
                b'`' => {
                    self.pos += 1;
                    cauda = true;
                    break;
                }
                b'$' if self.olhar(1) == b'{' => {
                    self.pos += 2;
                    cauda = false;
                    break;
                }
                b'\\' => {
                    let p = self.pos;
                    self.pos += 1;
                    let r = self.escape(true);
                    match r {
                        Ok(e) => {
                            if let Some(cz) = cozido.as_mut() {
                                match e {
                                    Escape::Unidades(a, b) => {
                                        cz.push(a);
                                        if let Some(b) = b {
                                            cz.push(b);
                                        }
                                    }
                                    Escape::Continuacao | Escape::OctalLegado(_) => {}
                                }
                            }
                        }
                        Err(_) => {
                            // Escape inválido: permitido só em template marcado (cozido = undefined).
                            cozido = None;
                            self.pos = p + 1;
                            if self.pos < self.b.len() {
                                self.avancar_char();
                            }
                        }
                    }
                    // O cru é o texto do fonte, com CR/CRLF virando LF.
                    empurrar_cru(&mut cru, &self.fonte[p..self.pos]);
                }
                b'\r' => {
                    self.pos += 1;
                    if self.olhar(0) == b'\n' {
                        self.pos += 1;
                    }
                    cru.push(u16::from(b'\n'));
                    if let Some(cz) = cozido.as_mut() {
                        cz.push(u16::from(b'\n'));
                    }
                }
                _ => {
                    let ch = self.char_aqui().unwrap_or('\0');
                    self.avancar_char();
                    let mut buf = [0u16; 2];
                    let u = ch.encode_utf16(&mut buf);
                    cru.extend_from_slice(u);
                    if let Some(cz) = cozido.as_mut() {
                        cz.extend_from_slice(u);
                    }
                }
            }
        }
        s.tipo = Tipo::Template(Template { cabeca, cauda });
        self.templates.push(ParteTemplate { cozido, cru });
        s.dado = Dado::Template(self.templates.len() as u32 - 1);
        Ok(())
    }

    /// `/corpo/flags`. O corpo só é validado pelo compilador de regex; aqui basta achar o fim
    /// (classes `[...]` podem conter `/`).
    fn ler_regex(&mut self, s: &mut Simbolo) -> R<()> {
        let ini = self.pos;
        self.pos += 1;
        let corpo = self.pos;
        let mut classe = false;
        loop {
            if self.pos >= self.b.len() || self.fim_de_linha_aqui() {
                return self.erro(ini, "expressão regular sem fechar");
            }
            match self.b[self.pos] {
                b'\\' => {
                    self.pos += 1;
                    if self.pos >= self.b.len() || self.fim_de_linha_aqui() {
                        return self.erro(ini, "expressão regular sem fechar");
                    }
                    self.avancar_char();
                }
                b'[' => {
                    classe = true;
                    self.pos += 1;
                }
                b']' => {
                    classe = false;
                    self.pos += 1;
                }
                b'/' if !classe => break,
                _ => self.avancar_char(),
            }
        }
        let fim_corpo = self.pos;
        self.pos += 1;
        let flags = self.pos;
        while self.pos < self.b.len() {
            let ch = self.char_aqui().unwrap_or('\0');
            if ch == '\\' {
                return self.erro(self.pos, "escape nas flags da expressão regular");
            }
            if !parte_de_nome(ch) {
                break;
            }
            self.avancar_char();
        }
        s.tipo = Tipo::Regex;
        s.dado = Dado::Regex { corpo: (corpo as u32, fim_corpo as u32), flags: (flags as u32, self.pos as u32) };
        Ok(())
    }
}

enum Escape {
    Unidades(u16, Option<u16>),
    Continuacao,
    OctalLegado(u16),
}

fn unidades(v: u32) -> Escape {
    if v < 0x10000 {
        Escape::Unidades(v as u16, None)
    } else {
        let v = v - 0x10000;
        Escape::Unidades(0xD800 + (v >> 10) as u16, Some(0xDC00 + (v & 0x3FF) as u16))
    }
}

fn empurrar_cru(cru: &mut Vec<u16>, texto: &str) {
    let mut it = texto.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\r' {
            if it.peek() == Some(&'\n') {
                it.next();
            }
            cru.push(u16::from(b'\n'));
        } else {
            let mut buf = [0u16; 2];
            cru.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
}

/// Dígitos em base 2/8/16 → texto decimal (BigInt sem limite de tamanho).
fn bigint_para_decimal(digitos: &str, base: u32) -> String {
    // Em "membros" de 10^9, do menos significativo para o mais.
    let mut membros: Vec<u32> = vec![0];
    for c in digitos.chars() {
        let mut vai = u64::from(c.to_digit(base).unwrap_or(0));
        for m in &mut membros {
            let v = u64::from(*m) * u64::from(base) + vai;
            *m = (v % 1_000_000_000) as u32;
            vai = v / 1_000_000_000;
        }
        while vai > 0 {
            membros.push((vai % 1_000_000_000) as u32);
            vai /= 1_000_000_000;
        }
    }
    let mut s = membros.last().unwrap().to_string();
    for m in membros.iter().rev().skip(1) {
        s.push_str(&format!("{m:09}"));
    }
    s
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::atomos::palavras;

    /// Lê tudo, decidindo `/` pelo símbolo anterior (como faria a sintaxe na maioria dos casos).
    fn todos(fonte: &str) -> Vec<Simbolo> {
        let mut l = Lexico::novo(fonte, false);
        let mut v: Vec<Simbolo> = Vec::new();
        loop {
            let obj = match v.last().map(|s| s.tipo) {
                Some(Tipo::Nome | Tipo::Numero | Tipo::Texto | Tipo::Regex | Tipo::BigInt) => Objetivo::Divisao,
                Some(Tipo::Pont(Pont::FechaParen | Pont::FechaColchete | Pont::FechaChave)) => Objetivo::Divisao,
                _ => Objetivo::Regex,
            };
            let s = l.proximo(obj).unwrap();
            if s.tipo == Tipo::Fim {
                return v;
            }
            v.push(s);
        }
    }

    fn numero(fonte: &str) -> f64 {
        match Lexico::novo(fonte, false).proximo(Objetivo::Regex).unwrap().dado {
            Dado::Numero(n) => n,
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn nomes_viram_atomos_e_palavras_tem_numero_fixo() {
        let mut l = Lexico::novo("if (x) yield x", false);
        let s = l.proximo(Objetivo::Regex).unwrap();
        assert_eq!(s.dado, Dado::Atomo(palavras::IF));
        assert_eq!(l.proximo(Objetivo::Regex).unwrap().tipo, Tipo::Pont(Pont::AbreParen));
        let x = l.proximo(Objetivo::Regex).unwrap().dado;
        l.proximo(Objetivo::Divisao).unwrap();
        assert_eq!(l.proximo(Objetivo::Regex).unwrap().dado, Dado::Atomo(palavras::YIELD));
        assert_eq!(l.proximo(Objetivo::Regex).unwrap().dado, x);
    }

    #[test]
    fn nome_com_escape_e_unicode() {
        let mut l = Lexico::novo("\\u0069f ação \\u{1F600}", false);
        let s = l.proximo(Objetivo::Regex).unwrap();
        // `if` é o nome "if", mas escapado (não vale como palavra reservada).
        assert_eq!(s.dado, Dado::Atomo(palavras::IF));
        assert!(s.escapado);
        let s = l.proximo(Objetivo::Regex).unwrap();
        let Dado::Atomo(a) = s.dado else { panic!() };
        assert_eq!(l.atomos.texto(a), "ação");
        // U+1F600 não é ID_Start.
        assert!(l.proximo(Objetivo::Regex).is_err());
    }

    #[test]
    fn numeros() {
        assert_eq!(numero("0x1F"), 31.0);
        assert_eq!(numero("0b1010"), 10.0);
        assert_eq!(numero("0o17"), 15.0);
        assert_eq!(numero("017"), 15.0);
        assert_eq!(numero("08"), 8.0);
        assert_eq!(numero("1_000_000"), 1e6);
        assert_eq!(numero(".5e3"), 500.0);
        assert_eq!(numero("7."), 7.0);
        assert_eq!(numero("1e-7"), 1e-7);
        assert!(Lexico::novo("3in", false).proximo(Objetivo::Regex).is_err());
        assert!(Lexico::novo("1__0", false).proximo(Objetivo::Regex).is_err());
        assert!(Lexico::novo("1_", false).proximo(Objetivo::Regex).is_err());
        assert!(Lexico::novo("0x", false).proximo(Objetivo::Regex).is_err());
        let mut l = Lexico::novo("0xFFn 12n", false);
        let s = l.proximo(Objetivo::Regex).unwrap();
        assert_eq!(s.tipo, Tipo::BigInt);
        let Dado::Texto(i) = s.dado else { panic!() };
        assert_eq!(String::from_utf16_lossy(l.texto(i)), "255");
    }

    #[test]
    fn textos_sem_escape_nao_copiam() {
        let fonte = r#"'abc' "a\tb\u{1F600}\x41\101""#;
        let mut l = Lexico::novo(fonte, false);
        let s = l.proximo(Objetivo::Regex).unwrap();
        assert_eq!(s.dado, Dado::Faixa(1, 4));
        let s = l.proximo(Objetivo::Regex).unwrap();
        assert!(s.escapado, "\\101 é octal legado");
        let Dado::Texto(i) = s.dado else { panic!() };
        assert_eq!(String::from_utf16(l.texto(i)).unwrap(), "a\tb\u{1F600}AA");
        assert!(Lexico::novo("'a\nb'", false).proximo(Objetivo::Regex).is_err());
        // Continuação de linha.
        let mut l = Lexico::novo("'a\\\nb'", false);
        let Dado::Texto(i) = l.proximo(Objetivo::Regex).unwrap().dado else { panic!() };
        assert_eq!(String::from_utf16(l.texto(i)).unwrap(), "ab");
    }

    #[test]
    fn pontuadores_mais_longos_primeiro() {
        let v = todos("a >>>= b ?? c ?.d a?.5:0 x **= 2 ... =>");
        let p: Vec<Pont> = v.iter().filter_map(|s| if let Tipo::Pont(p) = s.tipo { Some(p) } else { None }).collect();
        use Pont::*;
        assert_eq!(p, vec![DeslDirSemSinalAtrib, Coalesce, InterrogacaoPonto, Interrogacao, DoisPontos, PotenciaAtrib, Reticencias, Seta]);
    }

    #[test]
    fn barra_depende_do_objetivo() {
        let v = todos("a = b / c / d; e = /[/]x\\/y/gi.test(f)");
        assert_eq!(v.iter().filter(|s| s.tipo == Tipo::Pont(Pont::Barra)).count(), 2);
        let r = v.iter().find(|s| s.tipo == Tipo::Regex).unwrap();
        let Dado::Regex { corpo, flags } = r.dado else { panic!() };
        let fonte = "a = b / c / d; e = /[/]x\\/y/gi.test(f)";
        assert_eq!(&fonte[corpo.0 as usize..corpo.1 as usize], "[/]x\\/y");
        assert_eq!(&fonte[flags.0 as usize..flags.1 as usize], "gi");
    }

    #[test]
    fn templates_com_substituicao() {
        let fonte = "`a${x}b\\u0041${y}c\r\nd` `\\unicode`";
        let mut l = Lexico::novo(fonte, false);
        let s = l.proximo(Objetivo::Regex).unwrap();
        assert_eq!(s.tipo, Tipo::Template(Template { cabeca: true, cauda: false }));
        l.proximo(Objetivo::Regex).unwrap(); // x
        let chave = l.proximo(Objetivo::Divisao).unwrap();
        assert_eq!(chave.tipo, Tipo::Pont(Pont::FechaChave));
        let s = l.continuar_template(&chave).unwrap();
        assert_eq!(s.tipo, Tipo::Template(Template { cabeca: false, cauda: false }));
        let Dado::Template(i) = s.dado else { panic!() };
        assert_eq!(String::from_utf16(l.template(i).cozido.as_ref().unwrap()).unwrap(), "bA");
        assert_eq!(String::from_utf16(&l.template(i).cru).unwrap(), "b\\u0041");
        l.proximo(Objetivo::Regex).unwrap(); // y
        let chave = l.proximo(Objetivo::Divisao).unwrap();
        let s = l.continuar_template(&chave).unwrap();
        assert_eq!(s.tipo, Tipo::Template(Template { cabeca: false, cauda: true }));
        let Dado::Template(i) = s.dado else { panic!() };
        assert_eq!(String::from_utf16(&l.template(i).cru).unwrap(), "c\nd");
        // Escape inválido: cozido indefinido (vale em template marcado).
        let s = l.proximo(Objetivo::Regex).unwrap();
        let Dado::Template(i) = s.dado else { panic!() };
        assert!(l.template(i).cozido.is_none());
        assert_eq!(String::from_utf16(&l.template(i).cru).unwrap(), "\\unicode");
    }

    #[test]
    fn quebra_de_linha_e_comentarios() {
        let v = todos("a /* x\n */ b // c\nc /* */ d\n<!-- velho\n--> também\ne");
        let quebras: Vec<bool> = v.iter().map(|s| s.quebra_antes).collect();
        assert_eq!(&quebras[..5], &[false, true, true, false, true]);
        assert_eq!(v[4].tipo, Tipo::Nome);
        assert!(Lexico::novo("/* sem fim", false).proximo(Objetivo::Regex).is_err());
        // Módulo: `<!--` é `<` `!` `--`.
        let mut l = Lexico::novo("<!--", true);
        assert_eq!(l.proximo(Objetivo::Divisao).unwrap().tipo, Tipo::Pont(Pont::Menor));
    }

    #[test]
    fn hashbang_e_privados() {
        let mut l = Lexico::novo("#!/usr/bin/env node\nthis.#campo", false);
        assert_eq!(l.proximo(Objetivo::Regex).unwrap().dado, Dado::Atomo(palavras::THIS));
        l.proximo(Objetivo::Divisao).unwrap();
        let s = l.proximo(Objetivo::Regex).unwrap();
        assert_eq!(s.tipo, Tipo::NomePrivado);
        let Dado::Atomo(a) = s.dado else { panic!() };
        assert_eq!(l.atomos.texto(a), "campo");
    }

    #[test]
    fn bigint_de_base_grande() {
        assert_eq!(bigint_para_decimal("ffffffffffffffffffff", 16), "1208925819614629174706175");
    }
}
