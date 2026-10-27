//! Runtime do motor Falcão: valores de 32 bits, gaiola, formas, objetos, coletor e o
//! intérprete N1 (`ARQUITETURA.md`, seções 7 a 9, 13 e 14).

mod builtins;
pub mod coletor;
pub mod conversoes;
pub mod datas;
pub mod formas;
pub mod funcoes;
pub mod gaiola;
pub mod grande;
pub mod interpretador;
pub mod intrinsecos;
pub mod modulos;
pub mod motor;
pub mod nomes;
pub mod numeros;
pub mod objetos;
pub mod operadores;
pub mod promessas;
pub mod propriedades;
pub mod regex;
pub mod textos;
pub mod valor;

pub use funcoes::{Args, Res};
pub use gaiola::Gaiola;
pub use motor::Motor;
pub use valor::Valor;
