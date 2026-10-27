//! Texto legível do bytecode (para testes e para o F12).

use crate::funcao::{Constante, FuncaoBytecode, Script};
use crate::instrucoes::{ler_operando, Op, Operando};
use std::fmt::Write;

pub fn desmontar(script: &Script, f: &FuncaoBytecode) -> String {
    let mut s = String::new();
    let mut pc = 0usize;
    let c = &f.codigo;
    while pc < c.len() {
        let inicio = pc;
        let mut escala = 1;
        let mut op = Op::de_byte(c[pc]).expect("instrução inválida");
        while matches!(op, Op::Largo | Op::Largo4) {
            escala = if op == Op::Largo { 2 } else { 4 };
            pc += 1;
            op = Op::de_byte(c[pc]).expect("instrução inválida");
        }
        pc += 1;
        let _ = write!(s, "{inicio:4}  {}", op.nome());
        for (i, &t) in op.operandos().iter().enumerate() {
            let (v, n) = ler_operando(c, pc, t, escala);
            pc += n;
            let sep = if i == 0 { " " } else { ", " };
            let _ = match t {
                Operando::R => write!(s, "{sep}r{v}"),
                Operando::K => write!(s, "{sep}k{v}{}", constante(script, f, v)),
                Operando::C => write!(s, "{sep}c{v}"),
                Operando::N => write!(s, "{sep}{v}"),
                Operando::I => write!(s, "{sep}{}", v as i32),
                Operando::A => write!(s, "{sep}@{v}"),
            };
        }
        s.push('\n');
    }
    s
}

fn constante(script: &Script, f: &FuncaoBytecode, k: u32) -> String {
    let texto = |i: u32| String::from_utf16_lossy(&script.textos[i as usize]);
    match f.constantes.get(k as usize) {
        Some(Constante::Numero(n)) => format!("({n})"),
        Some(Constante::Texto(i)) => format!("({:?})", texto(*i)),
        Some(Constante::Nome(i)) => format!("({})", texto(*i)),
        Some(Constante::Funcao(i)) => format!("(função {i})"),
        _ => String::new(),
    }
}
