//! Emissor: escreve instruções com a largura certa dos operandos e resolve os rótulos de
//! salto no fim.

use crate::instrucoes::{Op, Operando};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rotulo(pub u32);

#[derive(Default)]
pub struct Emissor {
    pub codigo: Vec<u8>,
    /// Endereço de cada rótulo (u32::MAX = ainda não posto).
    rotulos: Vec<u32>,
    /// (posição do operando A no código, rótulo)
    pendentes: Vec<(usize, Rotulo)>,
    pub caches: u32,
    pub posicoes: Vec<(u32, u32)>,
}

/// Valor de um operando na hora de emitir.
#[derive(Clone, Copy, Debug)]
pub enum Arg {
    U(u32),
    I(i32),
    Para(Rotulo),
    /// Cache novo (o emissor numera).
    Cache,
}

impl Emissor {
    pub fn novo_rotulo(&mut self) -> Rotulo {
        self.rotulos.push(u32::MAX);
        Rotulo(self.rotulos.len() as u32 - 1)
    }

    pub fn por(&mut self, r: Rotulo) {
        self.rotulos[r.0 as usize] = self.codigo.len() as u32;
    }

    pub fn aqui(&self) -> u32 {
        self.codigo.len() as u32
    }

    pub fn endereco(&self, r: Rotulo) -> u32 {
        self.rotulos[r.0 as usize]
    }

    /// Anota a posição no fonte da próxima instrução.
    pub fn posicao(&mut self, fonte: u32) {
        let pc = self.codigo.len() as u32;
        match self.posicoes.last_mut() {
            Some(u) if u.0 == pc => u.1 = fonte,
            Some(u) if u.1 == fonte => {}
            _ => self.posicoes.push((pc, fonte)),
        }
    }

    pub fn emitir(&mut self, op: Op, args: &[Arg]) {
        let tipos = op.operandos();
        debug_assert_eq!(tipos.len(), args.len(), "{op:?}");
        let mut valores = [0u32; 4];
        let mut maior = 0u32;
        for (i, (&t, &a)) in tipos.iter().zip(args).enumerate() {
            let v = match a {
                Arg::U(u) => u,
                Arg::I(i) => i as u32,
                Arg::Para(_) => 0,
                Arg::Cache => {
                    self.caches += 1;
                    self.caches - 1
                }
            };
            valores[i] = v;
            if !matches!(t, Operando::I | Operando::A) {
                maior = maior.max(v);
            }
        }
        let escala = if maior > 0xFFFF {
            self.codigo.push(Op::Largo4 as u8);
            4
        } else if maior > 0xFF {
            self.codigo.push(Op::Largo as u8);
            2
        } else {
            1
        };
        self.codigo.push(op as u8);
        for (i, (&t, &a)) in tipos.iter().zip(args).enumerate() {
            let largura = if matches!(t, Operando::I | Operando::A) { 4 } else { escala };
            if let Arg::Para(r) = a {
                self.pendentes.push((self.codigo.len(), r));
            }
            let v = valores[i];
            match largura {
                1 => self.codigo.push(v as u8),
                2 => self.codigo.extend_from_slice(&(v as u16).to_le_bytes()),
                _ => self.codigo.extend_from_slice(&v.to_le_bytes()),
            }
        }
    }

    /// Resolve os saltos. Pânico se um rótulo usado não foi posto (erro do gerador).
    pub fn terminar(&mut self) {
        for &(pos, r) in &self.pendentes {
            let alvo = self.rotulos[r.0 as usize];
            assert!(alvo != u32::MAX, "rótulo {} sem lugar", r.0);
            self.codigo[pos..pos + 4].copy_from_slice(&alvo.to_le_bytes());
        }
        self.pendentes.clear();
    }
}
