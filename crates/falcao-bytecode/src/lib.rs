//! Gerador de bytecode do Falcão (`ARQUITETURA.md`, seção 6): árvore + escopos → [`Script`].

pub mod compilador;
pub mod desmontar;
pub mod emissor;
pub mod funcao;
pub mod instrucoes;

pub use funcao::{Constante, Excecao, Exportacao, Flags, FuncaoBytecode, InfoModulo, Script};
pub use instrucoes::{Op, Operando};
pub use compilador::{compilar, ErroCompilacao};
