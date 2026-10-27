//! Átomos: cada nome distinto vira um número de 32 bits, uma vez só. A sintaxe compara
//! átomos (nunca texto), e o runtime usa os mesmos números como chaves de propriedade.
//! As palavras do JS são os primeiros átomos, em ordem fixa (`palavras::*`).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Hash rápido para chaves curtas (nomes do JS): multiplica e gira, 8 bytes por vez.
#[derive(Default, Clone, Copy)]
pub struct HashRapido(u64);

impl Hasher for HashRapido {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        let mut h = self.0;
        let mut pedacos = bytes.chunks_exact(8);
        for p in &mut pedacos {
            let v = u64::from_le_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]);
            h = (h.rotate_left(5) ^ v).wrapping_mul(K);
        }
        for &b in pedacos.remainder() {
            h = (h.rotate_left(5) ^ u64::from(b)).wrapping_mul(K);
        }
        self.0 = h;
    }

    #[inline]
    fn write_u8(&mut self, b: u8) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type MapaRapido<K, V> = HashMap<K, V, BuildHasherDefault<HashRapido>>;

/// Número de um nome internado.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Atomo(pub u32);

macro_rules! palavras {
    ($($nome:ident = $texto:literal),* $(,)?) => {
        /// Palavras do JS (reservadas e contextuais), na ordem em que são internadas.
        pub mod palavras {
            use super::Atomo;
            palavras!(@const 0u32; $($nome)*);
        }
        const TEXTOS_PALAVRAS: &[&str] = &[$($texto),*];
    };
    (@const $n:expr; $nome:ident $($resto:ident)*) => {
        pub const $nome: Atomo = Atomo($n);
        palavras!(@const $n + 1; $($resto)*);
    };
    (@const $n:expr;) => {
        /// Quantidade de palavras fixas.
        pub const TOTAL: u32 = $n;
    };
}

palavras! {
    AWAIT = "await", BREAK = "break", CASE = "case", CATCH = "catch", CLASS = "class",
    CONST = "const", CONTINUE = "continue", DEBUGGER = "debugger", DEFAULT = "default",
    DELETE = "delete", DO = "do", ELSE = "else", ENUM = "enum", EXPORT = "export",
    EXTENDS = "extends", FALSE = "false", FINALLY = "finally", FOR = "for",
    FUNCTION = "function", IF = "if", IMPORT = "import", IN = "in", INSTANCEOF = "instanceof",
    NEW = "new", NULL = "null", RETURN = "return", SUPER = "super", SWITCH = "switch",
    THIS = "this", THROW = "throw", TRUE = "true", TRY = "try", TYPEOF = "typeof", VAR = "var",
    VOID = "void", WHILE = "while", WITH = "with", YIELD = "yield",
    // Reservadas só no modo estrito.
    IMPLEMENTS = "implements", INTERFACE = "interface", LET = "let", PACKAGE = "package",
    PRIVATE = "private", PROTECTED = "protected", PUBLIC = "public", STATIC = "static",
    // Contextuais.
    AS = "as", ASYNC = "async", FROM = "from", GET = "get", OF = "of", SET = "set",
    TARGET = "target", META = "meta", ARGUMENTS = "arguments", EVAL = "eval",
    CONSTRUCTOR = "constructor", PROTOTYPE = "prototype", USE_STRICT = "use strict",
    LENGTH = "length", UNDEFINED = "undefined", ACCESSOR = "accessor",
}

/// Tabela de átomos de um script (ou de um contexto inteiro, compartilhada entre scripts).
pub struct Atomos {
    mapa: MapaRapido<Box<str>, Atomo>,
    textos: Vec<Box<str>>,
}

impl Default for Atomos {
    fn default() -> Self {
        Self::novo()
    }
}

impl Atomos {
    /// Tabela com as palavras do JS já internadas (`palavras::*`).
    pub fn novo() -> Self {
        let mut a = Self { mapa: MapaRapido::default(), textos: Vec::with_capacity(1024) };
        for t in TEXTOS_PALAVRAS {
            a.internar(t);
        }
        debug_assert_eq!(a.textos.len() as u32, palavras::TOTAL);
        a
    }

    /// Número do nome `texto`, criando se for novo.
    #[inline]
    pub fn internar(&mut self, texto: &str) -> Atomo {
        if let Some(&a) = self.mapa.get(texto) {
            return a;
        }
        let a = Atomo(self.textos.len() as u32);
        let caixa: Box<str> = texto.into();
        self.textos.push(caixa.clone());
        self.mapa.insert(caixa, a);
        a
    }

    /// Número do nome, se já existe.
    pub fn procurar(&self, texto: &str) -> Option<Atomo> {
        self.mapa.get(texto).copied()
    }

    /// Texto de um átomo.
    pub fn texto(&self, a: Atomo) -> &str {
        &self.textos[a.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.textos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textos.is_empty()
    }
}

/// Palavras reservadas em qualquer modo (não podem ser nome de variável).
pub fn reservada(a: Atomo) -> bool {
    a.0 <= palavras::YIELD.0 && a != palavras::AWAIT && a != palavras::YIELD
}

/// Reservadas a mais no modo estrito (`let`, `static`, `yield`, `implements`...).
pub fn reservada_estrita(a: Atomo) -> bool {
    reservada(a) || a == palavras::YIELD || (palavras::IMPLEMENTS.0..=palavras::STATIC.0).contains(&a.0)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn palavras_tem_numeros_fixos() {
        let mut a = Atomos::novo();
        assert_eq!(a.internar("if"), palavras::IF);
        assert_eq!(a.internar("constructor"), palavras::CONSTRUCTOR);
        assert_eq!(a.texto(palavras::FUNCTION), "function");
        let x = a.internar("x");
        assert_eq!(x.0, palavras::TOTAL);
        assert_eq!(a.internar("x"), x);
        assert!(reservada(palavras::IF) && !reservada(palavras::LET) && !reservada(palavras::YIELD));
        assert!(reservada_estrita(palavras::LET) && reservada_estrita(palavras::YIELD));
        assert!(!reservada_estrita(palavras::ASYNC));
    }
}
