//! Classes de caractere do JS (ECMA-262 §12.2, §12.3 e §12.7), com caminho rápido para ASCII.

use crate::unicode_tabelas::{ESPACO_ZS, ID_CONTINUE, ID_START};

fn na_tabela(t: &[(u32, u32)], c: char) -> bool {
    let c = c as u32;
    t.binary_search_by(|&(a, b)| {
        if b < c {
            std::cmp::Ordering::Less
        } else if a > c {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

/// Começo de nome: `ID_Start`, `$` ou `_`.
pub(crate) fn inicio_de_nome(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphabetic() || c == '$' || c == '_'
    } else {
        na_tabela(ID_START, c)
    }
}

/// Resto de nome: `ID_Continue`, `$`, ZWNJ e ZWJ (`_` já está em `ID_Continue`).
pub(crate) fn parte_de_nome(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || c == '$' || c == '_'
    } else {
        c == '\u{200C}' || c == '\u{200D}' || na_tabela(ID_CONTINUE, c)
    }
}

/// Espaço em branco: TAB, VT, FF, ZWNBSP e a categoria `Zs`.
pub(crate) fn espaco(c: char) -> bool {
    matches!(c, '\t' | '\u{0B}' | '\u{0C}' | ' ' | '\u{FEFF}') || (!c.is_ascii() && na_tabela(ESPACO_ZS, c))
}

/// Fim de linha: LF, CR, LS e PS.
pub(crate) fn fim_de_linha(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}
