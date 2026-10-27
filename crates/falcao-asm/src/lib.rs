//! Montador x86-64 e memória executável do Falcão: a "caneta" do JIT N2 (ARQUITETURA.md, seção 10).
//!
//! - [`Montador`] escreve código de máquina num `Vec<u8>`: mov, aritmética, saltos com rótulos,
//!   SSE2 para doubles. Só o que um JIT de base precisa; nada de dependências.
//! - [`CodigoExecutavel`] põe os bytes numa página só leitura + execução (W^X: nunca gravável e
//!   executável ao mesmo tempo) e chama o código. Todo `unsafe` do crate mora em `memoria.rs`.
//!
//! Rótulos: salto para trás usa `rel8` quando cabe; salto para frente usa sempre `rel32` (não se
//! sabe a distância ainda, e reescrever o tamanho deslocaria o código já emitido). Rótulo usado e
//! nunca ligado é erro em [`Montador::finalizar`], nunca um salto para lixo.

mod memoria;
mod montador;

pub use memoria::CodigoExecutavel;
pub use montador::{Alu, Cond, Desloc, Escala, Mem, Montador, Reg, Rm, Rotulo, Sse, Tam, Unario, Xmm, MAX_CODIGO};

/// Erros do montador e da memória executável.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Erro {
    /// Rótulo usado por um salto e nunca ligado a uma posição.
    RotuloSemLugar(u32),
    /// O mesmo rótulo ligado duas vezes.
    RotuloLigadoDuasVezes(u32),
    /// Código maior que [`MAX_CODIGO`] (ou vazio, para a memória executável).
    TamanhoInvalido(usize),
    /// Plataforma sem memória executável implementada (só Linux x86-64 por ora).
    SemSuporte,
    /// Falha de `mmap`/`mprotect` (errno).
    Sistema(i32),
}
