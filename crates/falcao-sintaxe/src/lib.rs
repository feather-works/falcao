//! Sintaxe do motor Falcão: átomos, léxico (texto → símbolos) e sintaxe (símbolos → árvore).
//! Os escopos e a pré-análise entram em seguida (ver `ARQUITETURA.md`, seções 2 a 5).

pub mod arvore;
pub mod atomos;
mod comandos;
pub mod escopos;
pub mod lexico;
mod numeros;
pub mod sintaxe;
mod unicode;
mod unicode_tabelas;

pub use atomos::{palavras, Atomo, Atomos};
pub use lexico::{Dado, ErroLexico, Lexico, Objetivo, ParteTemplate, Pont, Simbolo, Template, Tipo};
pub use sintaxe::{analisar_modulo, analisar_script, ErroSintaxe};
pub use escopos::{analisar_escopos, Escopo, EscopoId, Escopos, Ref, TipoEscopo, TipoVinculo, Vinculo, VinculoId};
