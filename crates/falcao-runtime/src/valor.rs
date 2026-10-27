//! Valor de 32 bits (`ARQUITETURA.md`, seção 7):
//!
//! ```text
//!  ...xxxxxxx0   inteiro de 31 bits (bit 0 = 0)
//!  ...xxxxxxx1   referência: deslocamento em bytes dentro da gaiola (bit 0 = 1)
//! ```
//!
//! Os objetos ficam alinhados em 8 bytes, então o deslocamento sempre tem os 3 bits de baixo
//! zerados e o bit 0 pode marcar a referência. `undefined`, `null`, `true`, `false`, o buraco
//! de array e a string vazia são objetos fixos no começo da gaiola: comparar com eles é
//! comparar 32 bits.

/// Um valor do JS (ou um campo interno que guarda valor).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Valor(pub u32);

/// Maior e menor inteiro que cabem direto no valor.
pub const INT_MAX: i32 = (1 << 30) - 1;
pub const INT_MIN: i32 = -(1 << 30);

impl Valor {
    // Objetos fixos, nesta ordem no começo da gaiola (ver `gaiola::Gaiola::nova`).
    pub const UNDEFINED: Valor = Valor(8 | 1);
    pub const NULL: Valor = Valor(16 | 1);
    pub const TRUE: Valor = Valor(24 | 1);
    pub const FALSE: Valor = Valor(32 | 1);
    /// Buraco: item ausente de array, variável em TDZ, campo ainda não iniciado.
    pub const BURACO: Valor = Valor(40 | 1);
    /// String vazia.
    pub const VAZIA: Valor = Valor(48 | 1);
    /// Lista vazia (`extras`/`elementos` de objeto sem nada).
    pub const LISTA_VAZIA: Valor = Valor(64 | 1);
    pub const ZERO: Valor = Valor(0);

    #[inline(always)]
    pub const fn int(i: i32) -> Valor {
        debug_assert!(i >= INT_MIN && i <= INT_MAX);
        Valor((i << 1) as u32)
    }

    /// Inteiro se couber em 31 bits.
    #[inline(always)]
    pub fn int_se_cabe(i: i64) -> Option<Valor> {
        (i >= INT_MIN as i64 && i <= INT_MAX as i64).then(|| Valor::int(i as i32))
    }

    #[inline(always)]
    pub const fn e_int(self) -> bool {
        self.0 & 1 == 0
    }

    #[inline(always)]
    pub const fn e_ref(self) -> bool {
        self.0 & 1 == 1
    }

    #[inline(always)]
    pub const fn como_int(self) -> i32 {
        (self.0 as i32) >> 1
    }

    /// Deslocamento em bytes do objeto apontado.
    #[inline(always)]
    pub const fn deslocamento(self) -> u32 {
        self.0 & !1
    }

    #[inline(always)]
    pub const fn de_deslocamento(d: u32) -> Valor {
        Valor(d | 1)
    }

    #[inline(always)]
    pub fn bool(b: bool) -> Valor {
        if b {
            Valor::TRUE
        } else {
            Valor::FALSE
        }
    }

    #[inline(always)]
    pub fn e_nulo_ou_indefinido(self) -> bool {
        self == Valor::UNDEFINED || self == Valor::NULL
    }
}

impl std::fmt::Debug for Valor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Valor::UNDEFINED => write!(f, "undefined"),
            Valor::NULL => write!(f, "null"),
            Valor::TRUE => write!(f, "true"),
            Valor::FALSE => write!(f, "false"),
            Valor::BURACO => write!(f, "<buraco>"),
            Valor::VAZIA => write!(f, "\"\""),
            v if v.e_int() => write!(f, "{}", v.como_int()),
            v => write!(f, "@{}", v.deslocamento()),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn inteiros_e_referencias() {
        for i in [0, 1, -1, INT_MAX, INT_MIN, 12345, -9876] {
            let v = Valor::int(i);
            assert!(v.e_int() && !v.e_ref());
            assert_eq!(v.como_int(), i);
        }
        assert_eq!(Valor::int_se_cabe(INT_MAX as i64 + 1), None);
        let r = Valor::de_deslocamento(1024);
        assert!(r.e_ref() && r.deslocamento() == 1024);
        assert!(Valor::UNDEFINED.e_ref());
    }
}
