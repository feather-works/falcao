//! Nomes internados do runtime (chaves de propriedade). Ao contrário dos átomos da sintaxe,
//! aceitam qualquer UTF-16 (o JS permite surrogate solto em nome de propriedade). O bytecode não
//! guarda números de nome: cada script traz sua tabela de textos, traduzida para cá ao carregar,
//! e assim o mesmo bytecode serve a contextos diferentes.

use crate::formas::Chave;
use crate::valor::Valor;
use falcao_sintaxe::atomos::MapaRapido;

macro_rules! nomes_fixos {
    ($($nome:ident = $texto:literal),* $(,)?) => {
        /// Nomes que o runtime usa direto, com número fixo.
        pub mod fixos {
            use crate::formas::Chave;
            nomes_fixos!(@const 0u32; $($nome)*);
        }
        const TEXTOS_FIXOS: &[&str] = &[$($texto),*];
    };
    (@const $n:expr; $nome:ident $($resto:ident)*) => {
        pub const $nome: Chave = Chave($n);
        nomes_fixos!(@const $n + 1; $($resto)*);
    };
    (@const $n:expr;) => {
        pub const TOTAL: u32 = $n;
    };
}

nomes_fixos! {
    LENGTH = "length", PROTOTYPE = "prototype", CONSTRUCTOR = "constructor", NAME = "name",
    MESSAGE = "message", STACK = "stack", CAUSE = "cause", TO_STRING = "toString",
    VALUE_OF = "valueOf", TO_JSON = "toJSON", VALUE = "value", DONE = "done", NEXT = "next",
    RETURN = "return", THROW = "throw", THEN = "then", GET = "get", SET = "set",
    WRITABLE = "writable", ENUMERABLE = "enumerable", CONFIGURABLE = "configurable",
    CALLEE = "callee", CALLER = "caller", ARGUMENTS = "arguments", INDEX = "index",
    INPUT = "input", GROUPS = "groups", LAST_INDEX = "lastIndex", RAW = "raw",
    UNDEFINED = "undefined", NULL = "null", TRUE = "true", FALSE = "false", NAN = "NaN",
    INFINITY = "Infinity", OBJECT = "object", FUNCTION = "function", STRING = "string",
    NUMBER = "number", BOOLEAN = "boolean", SYMBOL = "symbol", BIGINT = "bigint",
    DEFAULT = "default", ERRORS = "errors", PROTO = "__proto__", GLOBAL_THIS = "globalThis",
    ANONIMO = "", HAS = "has", ADD = "add", SIZE = "size", KEYS = "keys", ENTRIES = "entries",
    VALUES = "values", FLAGS = "flags", SOURCE = "source", EXEC = "exec",
}

pub struct Nomes {
    mapa: MapaRapido<Box<[u16]>, u32>,
    textos: Vec<Box<[u16]>>,
    /// Texto do nome já alocado na gaiola (criado na primeira vez que o JS o vê como valor).
    pub valores: Vec<Valor>,
}

impl Default for Nomes {
    fn default() -> Self {
        Self::novo()
    }
}

impl Nomes {
    pub fn novo() -> Self {
        let mut n = Self { mapa: MapaRapido::default(), textos: Vec::with_capacity(4096), valores: Vec::with_capacity(4096) };
        for t in TEXTOS_FIXOS {
            n.internar_str(t);
        }
        debug_assert_eq!(n.textos.len() as u32, fixos::TOTAL);
        n
    }

    pub fn internar(&mut self, texto: &[u16]) -> Chave {
        if let Some(&c) = self.mapa.get(texto) {
            return Chave(c);
        }
        let c = self.textos.len() as u32;
        assert!(c < Chave::BIT_SIMBOLO, "nomes demais");
        let caixa: Box<[u16]> = texto.into();
        self.textos.push(caixa.clone());
        self.valores.push(Valor::UNDEFINED);
        self.mapa.insert(caixa, c);
        Chave(c)
    }

    pub fn internar_str(&mut self, texto: &str) -> Chave {
        let u: Vec<u16> = texto.encode_utf16().collect();
        self.internar(&u)
    }

    pub fn procurar(&self, texto: &[u16]) -> Option<Chave> {
        self.mapa.get(texto).map(|&c| Chave(c))
    }

    pub fn texto(&self, c: Chave) -> &[u16] {
        &self.textos[c.0 as usize]
    }

    pub fn texto_rust(&self, c: Chave) -> String {
        String::from_utf16_lossy(self.texto(c))
    }

    /// Hash estável do texto (para a identidade das formas).
    pub fn hash_estavel(&self, c: Chave) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &u in self.texto(c) {
            h = (h ^ u64::from(u)).wrapping_mul(0x100_0000_01b3);
        }
        h
    }

    pub fn len(&self) -> usize {
        self.textos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textos.is_empty()
    }
}
