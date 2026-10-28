//! Codificação x86-64: prefixos, REX, ModRM, SIB, deslocamentos e rótulos.

use crate::Erro;

/// Maior código que um [`Montador`] entrega (16 MB): os `rel32` sempre cabem.
pub const MAX_CODIGO: usize = 16 << 20;

/// Registradores de uso geral; o número é o da codificação (bit 3 vai no REX).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[rustfmt::skip]
pub enum Reg { Rax, Rcx, Rdx, Rbx, Rsp, Rbp, Rsi, Rdi, R8, R9, R10, R11, R12, R13, R14, R15 }

impl Reg {
    pub const TODOS: [Reg; 16] = [
        Reg::Rax, Reg::Rcx, Reg::Rdx, Reg::Rbx, Reg::Rsp, Reg::Rbp, Reg::Rsi, Reg::Rdi,
        Reg::R8, Reg::R9, Reg::R10, Reg::R11, Reg::R12, Reg::R13, Reg::R14, Reg::R15,
    ];

    fn n(self) -> u8 {
        self as u8
    }
}

/// Registradores SSE.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[rustfmt::skip]
pub enum Xmm { X0, X1, X2, X3, X4, X5, X6, X7, X8, X9, X10, X11, X12, X13, X14, X15 }

impl Xmm {
    pub const TODOS: [Xmm; 16] = [
        Xmm::X0, Xmm::X1, Xmm::X2, Xmm::X3, Xmm::X4, Xmm::X5, Xmm::X6, Xmm::X7,
        Xmm::X8, Xmm::X9, Xmm::X10, Xmm::X11, Xmm::X12, Xmm::X13, Xmm::X14, Xmm::X15,
    ];
}

/// Largura da operação inteira.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tam {
    B32,
    B64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Escala {
    E1,
    E2,
    E4,
    E8,
}

/// Endereço `[base + indice*escala + desl]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mem {
    pub base: Reg,
    /// O índice não pode ser `rsp` (a codificação usa esse número para "sem índice").
    pub indice: Option<(Reg, Escala)>,
    pub desl: i32,
}

impl Mem {
    pub fn base(base: Reg, desl: i32) -> Mem {
        Mem { base, indice: None, desl }
    }

    pub fn indexado(base: Reg, indice: Reg, escala: Escala, desl: i32) -> Mem {
        assert!(indice != Reg::Rsp, "rsp não pode ser índice");
        Mem { base, indice: Some((indice, escala)), desl }
    }
}

/// Operando registrador ou memória (o campo r/m do ModRM).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rm {
    R(Reg),
    M(Mem),
}

impl From<Reg> for Rm {
    fn from(r: Reg) -> Rm {
        Rm::R(r)
    }
}

impl From<Mem> for Rm {
    fn from(m: Mem) -> Rm {
        Rm::M(m)
    }
}

/// Condições de `jcc`/`setcc`/`cmovcc` (o número é o da codificação).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[rustfmt::skip]
pub enum Cond { O, No, B, Ae, E, Ne, Be, A, S, Ns, P, Np, L, Ge, Le, G }

impl Cond {
    pub const TODAS: [Cond; 16] = [
        Cond::O, Cond::No, Cond::B, Cond::Ae, Cond::E, Cond::Ne, Cond::Be, Cond::A,
        Cond::S, Cond::Ns, Cond::P, Cond::Np, Cond::L, Cond::Ge, Cond::Le, Cond::G,
    ];
}

/// Operações de dois operandos do grupo 1; o número é o `/n` do ModRM.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alu {
    Add = 0,
    Or = 1,
    And = 4,
    Sub = 5,
    Xor = 6,
    Cmp = 7,
}

/// Deslocamentos de bits; o número é o `/n`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Desloc {
    Shl = 4,
    Shr = 5,
    Sar = 7,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unario {
    Not,
    Neg,
    Inc,
    Dec,
    /// edx:eax / r sem sinal: quociente em eax, resto em edx (rdx:rax em 64 bits).
    Div,
    /// Idem, com sinal.
    Idiv,
}

/// Aritmética escalar de doubles (o byte é o opcode depois de `F2 0F`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sse {
    Add = 0x58,
    Mul = 0x59,
    Sub = 0x5C,
    Div = 0x5E,
}

/// Posição de destino de saltos; ligada uma vez com [`Montador::ligar`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rotulo(u32);

#[derive(Default)]
pub struct Montador {
    codigo: Vec<u8>,
    /// Posição de cada rótulo (None = ainda não ligado).
    rotulos: Vec<Option<usize>>,
    /// `rel32` por preencher: (onde está o campo, rótulo). O salto conta a partir do fim do campo.
    pendentes: Vec<(usize, u32)>,
    erro: Option<Erro>,
}

impl Montador {
    pub fn novo() -> Montador {
        Montador::default()
    }

    /// Bytes emitidos até agora.
    pub fn pos(&self) -> usize {
        self.codigo.len()
    }

    pub fn rotulo(&mut self) -> Rotulo {
        self.rotulos.push(None);
        Rotulo(self.rotulos.len() as u32 - 1)
    }

    /// Liga o rótulo à posição atual.
    pub fn ligar(&mut self, r: Rotulo) {
        let l = &mut self.rotulos[r.0 as usize];
        if l.is_some() {
            self.erro.get_or_insert(Erro::RotuloLigadoDuasVezes(r.0));
        }
        *l = Some(self.codigo.len());
    }

    /// Preenche os saltos para frente e entrega o código. Erro se algum rótulo usado ficou sem
    /// lugar, se um rótulo foi ligado duas vezes ou se o código passou de [`MAX_CODIGO`].
    pub fn finalizar(mut self) -> Result<Vec<u8>, Erro> {
        if let Some(e) = self.erro {
            return Err(e);
        }
        if self.codigo.len() > MAX_CODIGO {
            return Err(Erro::TamanhoInvalido(self.codigo.len()));
        }
        for &(campo, r) in &self.pendentes {
            let alvo = self.rotulos[r as usize].ok_or(Erro::RotuloSemLugar(r))?;
            let rel = alvo as i64 - (campo as i64 + 4);
            self.codigo[campo..campo + 4].copy_from_slice(&(rel as i32).to_le_bytes());
        }
        Ok(self.codigo)
    }

    // ---------------------------------------------------------------- codificação

    fn b(&mut self, x: u8) {
        self.codigo.push(x);
    }

    fn i32(&mut self, x: i32) {
        self.codigo.extend_from_slice(&x.to_le_bytes());
    }

    /// Prefixo legado, REX (se preciso), opcode, ModRM, SIB e deslocamento.
    /// `reg` é o campo reg do ModRM (registrador ou o `/n` da extensão de opcode).
    /// `rex_vazio`: força o REX (0x40) para os bytes `spl`/`bpl`/`sil`/`dil`.
    fn op(&mut self, prefixo: Option<u8>, w: bool, opcode: &[u8], reg: u8, rm: Rm, rex_vazio: bool) {
        if let Some(p) = prefixo {
            self.b(p);
        }
        let (x, bb) = match rm {
            Rm::R(r) => (0, r.n() >> 3),
            Rm::M(m) => (m.indice.map_or(0, |(i, _)| i.n() >> 3), m.base.n() >> 3),
        };
        let rex = 0x40 | u8::from(w) << 3 | (reg >> 3) << 2 | x << 1 | bb;
        if rex != 0x40 || rex_vazio {
            self.b(rex);
        }
        self.codigo.extend_from_slice(opcode);
        let reg = (reg & 7) << 3;
        match rm {
            Rm::R(r) => self.b(0xC0 | reg | (r.n() & 7)),
            Rm::M(m) => {
                let base = m.base.n() & 7;
                // rbp/r13 sem deslocamento seriam "[rip+disp32]"/"sem base": vão com disp8 0.
                let modo = if m.desl == 0 && base != 5 {
                    0x00
                } else if i8::try_from(m.desl).is_ok() {
                    0x40
                } else {
                    0x80
                };
                // rsp/r12 como base pedem SIB (o 100 no r/m quer dizer "tem SIB").
                if m.indice.is_some() || base == 4 {
                    self.b(modo | reg | 4);
                    let (i, e) = m.indice.map_or((4, 0), |(i, e)| (i.n() & 7, e as u8));
                    self.b(e << 6 | i << 3 | base);
                } else {
                    self.b(modo | reg | base);
                }
                match modo {
                    0x40 => self.b(m.desl as i8 as u8),
                    0x80 => self.i32(m.desl),
                    _ => {}
                }
            }
        }
    }

    /// Salto para um rótulo: `curto` = opcode da forma rel8 (só para trás e se couber).
    fn salto(&mut self, curto: Option<u8>, longo: &[u8], r: Rotulo) {
        if let (Some(c), Some(alvo)) = (curto, self.rotulos[r.0 as usize]) {
            let rel = alvo as i64 - (self.codigo.len() as i64 + 2);
            if let Ok(rel) = i8::try_from(rel) {
                self.b(c);
                self.b(rel as u8);
                return;
            }
        }
        self.codigo.extend_from_slice(longo);
        self.pendentes.push((self.codigo.len(), r.0));
        self.i32(0);
    }

    // ---------------------------------------------------------------- mov e memória

    pub fn mov(&mut self, t: Tam, dst: Reg, src: Reg) {
        self.op(None, t == Tam::B64, &[0x89], src.n(), Rm::R(dst), false);
    }

    /// `mov` de imediato: 32 bits (zera a parte alta), 64 com sinal estendido quando cabe em
    /// 32, senão `movabs`.
    pub fn mov_imm(&mut self, t: Tam, dst: Reg, imm: i64) {
        match t {
            Tam::B32 => {
                if dst.n() >= 8 {
                    self.b(0x41);
                }
                self.b(0xB8 + (dst.n() & 7));
                self.i32(imm as i32);
            }
            Tam::B64 if i32::try_from(imm).is_ok() => {
                self.op(None, true, &[0xC7], 0, Rm::R(dst), false);
                self.i32(imm as i32);
            }
            Tam::B64 => {
                self.b(0x48 | dst.n() >> 3);
                self.b(0xB8 + (dst.n() & 7));
                self.codigo.extend_from_slice(&imm.to_le_bytes());
            }
        }
    }

    pub fn carregar(&mut self, t: Tam, dst: Reg, m: Mem) {
        self.op(None, t == Tam::B64, &[0x8B], dst.n(), Rm::M(m), false);
    }

    pub fn guardar(&mut self, t: Tam, m: Mem, src: Reg) {
        self.op(None, t == Tam::B64, &[0x89], src.n(), Rm::M(m), false);
    }

    /// Guarda um imediato de 32 bits (estendido com sinal em 64).
    pub fn guardar_imm(&mut self, t: Tam, m: Mem, imm: i32) {
        self.op(None, t == Tam::B64, &[0xC7], 0, Rm::M(m), false);
        self.i32(imm);
    }

    pub fn lea(&mut self, dst: Reg, m: Mem) {
        self.op(None, true, &[0x8D], dst.n(), Rm::M(m), false);
    }

    /// Zero-extensão de 8 bits (`src` registrador ou memória).
    pub fn movzx8(&mut self, t: Tam, dst: Reg, src: impl Into<Rm>) {
        let src = src.into();
        let byte_alto = matches!(src, Rm::R(r) if (4..8).contains(&r.n()));
        self.op(None, t == Tam::B64, &[0x0F, 0xB6], dst.n(), src, byte_alto);
    }

    pub fn movzx16(&mut self, t: Tam, dst: Reg, src: impl Into<Rm>) {
        self.op(None, t == Tam::B64, &[0x0F, 0xB7], dst.n(), src.into(), false);
    }

    pub fn movsx8(&mut self, t: Tam, dst: Reg, src: impl Into<Rm>) {
        let src = src.into();
        let byte_alto = matches!(src, Rm::R(r) if (4..8).contains(&r.n()));
        self.op(None, t == Tam::B64, &[0x0F, 0xBE], dst.n(), src, byte_alto);
    }

    pub fn movsx16(&mut self, t: Tam, dst: Reg, src: impl Into<Rm>) {
        self.op(None, t == Tam::B64, &[0x0F, 0xBF], dst.n(), src.into(), false);
    }

    /// 32 → 64 bits com sinal.
    pub fn movsxd(&mut self, dst: Reg, src: impl Into<Rm>) {
        self.op(None, true, &[0x63], dst.n(), src.into(), false);
    }

    // ---------------------------------------------------------------- aritmética

    pub fn alu(&mut self, op: Alu, t: Tam, dst: Reg, src: Reg) {
        self.op(None, t == Tam::B64, &[(op as u8) << 3 | 1], src.n(), Rm::R(dst), false);
    }

    /// `dst op imm` (imm8 estendido quando cabe, senão imm32).
    pub fn alu_imm(&mut self, op: Alu, t: Tam, dst: Reg, imm: i32) {
        if let Ok(i) = i8::try_from(imm) {
            self.op(None, t == Tam::B64, &[0x83], op as u8, Rm::R(dst), false);
            self.b(i as u8);
        } else {
            self.op(None, t == Tam::B64, &[0x81], op as u8, Rm::R(dst), false);
            self.i32(imm);
        }
    }

    /// `dst op [m]`.
    pub fn alu_mem(&mut self, op: Alu, t: Tam, dst: Reg, m: Mem) {
        self.op(None, t == Tam::B64, &[(op as u8) << 3 | 3], dst.n(), Rm::M(m), false);
    }

    pub fn test(&mut self, t: Tam, a: Reg, b: Reg) {
        self.op(None, t == Tam::B64, &[0x85], b.n(), Rm::R(a), false);
    }

    pub fn test_imm(&mut self, t: Tam, a: Reg, imm: i32) {
        self.op(None, t == Tam::B64, &[0xF7], 0, Rm::R(a), false);
        self.i32(imm);
    }

    pub fn imul(&mut self, t: Tam, dst: Reg, src: Reg) {
        self.op(None, t == Tam::B64, &[0x0F, 0xAF], dst.n(), Rm::R(src), false);
    }

    /// `dst = src * imm`.
    pub fn imul_imm(&mut self, t: Tam, dst: Reg, src: Reg, imm: i32) {
        if let Ok(i) = i8::try_from(imm) {
            self.op(None, t == Tam::B64, &[0x6B], dst.n(), Rm::R(src), false);
            self.b(i as u8);
        } else {
            self.op(None, t == Tam::B64, &[0x69], dst.n(), Rm::R(src), false);
            self.i32(imm);
        }
    }

    pub fn unario(&mut self, op: Unario, t: Tam, r: Reg) {
        let (opcode, n) = match op {
            Unario::Not => (0xF7, 2),
            Unario::Neg => (0xF7, 3),
            Unario::Inc => (0xFF, 0),
            Unario::Dec => (0xFF, 1),
            Unario::Div => (0xF7, 6),
            Unario::Idiv => (0xF7, 7),
        };
        self.op(None, t == Tam::B64, &[opcode], n, Rm::R(r), false);
    }

    pub fn desloc(&mut self, op: Desloc, t: Tam, r: Reg, bits: u8) {
        self.op(None, t == Tam::B64, &[0xC1], op as u8, Rm::R(r), false);
        self.b(bits);
    }

    /// Desloca por `cl`.
    pub fn desloc_cl(&mut self, op: Desloc, t: Tam, r: Reg) {
        self.op(None, t == Tam::B64, &[0xD3], op as u8, Rm::R(r), false);
    }

    pub fn setcc(&mut self, c: Cond, r: Reg) {
        self.op(None, false, &[0x0F, 0x90 + c as u8], 0, Rm::R(r), (4..8).contains(&r.n()));
    }

    pub fn cmov(&mut self, c: Cond, t: Tam, dst: Reg, src: Reg) {
        self.op(None, t == Tam::B64, &[0x0F, 0x40 + c as u8], dst.n(), Rm::R(src), false);
    }

    // ---------------------------------------------------------------- pilha, chamadas e saltos

    pub fn push(&mut self, r: Reg) {
        if r.n() >= 8 {
            self.b(0x41);
        }
        self.b(0x50 + (r.n() & 7));
    }

    pub fn pop(&mut self, r: Reg) {
        if r.n() >= 8 {
            self.b(0x41);
        }
        self.b(0x58 + (r.n() & 7));
    }

    pub fn call(&mut self, r: Reg) {
        self.op(None, false, &[0xFF], 2, Rm::R(r), false);
    }

    pub fn call_rotulo(&mut self, l: Rotulo) {
        self.salto(None, &[0xE8], l);
    }

    pub fn ret(&mut self) {
        self.b(0xC3);
    }

    pub fn jmp(&mut self, l: Rotulo) {
        self.salto(Some(0xEB), &[0xE9], l);
    }

    pub fn jmp_reg(&mut self, r: Reg) {
        self.op(None, false, &[0xFF], 4, Rm::R(r), false);
    }

    pub fn jcc(&mut self, c: Cond, l: Rotulo) {
        self.salto(Some(0x70 + c as u8), &[0x0F, 0x80 + c as u8], l);
    }

    // ---------------------------------------------------------------- SSE2 (doubles)

    pub fn movsd(&mut self, dst: Xmm, src: Xmm) {
        self.op(Some(0xF2), false, &[0x0F, 0x10], dst as u8, Rm::R(Reg::TODOS[src as usize]), false);
    }

    pub fn movsd_carregar(&mut self, dst: Xmm, m: Mem) {
        self.op(Some(0xF2), false, &[0x0F, 0x10], dst as u8, Rm::M(m), false);
    }

    pub fn movsd_guardar(&mut self, m: Mem, src: Xmm) {
        self.op(Some(0xF2), false, &[0x0F, 0x11], src as u8, Rm::M(m), false);
    }

    /// Bits de um registrador de 64 bits para o xmm.
    pub fn movq_para_xmm(&mut self, dst: Xmm, src: Reg) {
        self.op(Some(0x66), true, &[0x0F, 0x6E], dst as u8, Rm::R(src), false);
    }

    pub fn movq_de_xmm(&mut self, dst: Reg, src: Xmm) {
        self.op(Some(0x66), true, &[0x0F, 0x7E], src as u8, Rm::R(dst), false);
    }

    pub fn sse(&mut self, op: Sse, dst: Xmm, src: Xmm) {
        self.op(Some(0xF2), false, &[0x0F, op as u8], dst as u8, Rm::R(Reg::TODOS[src as usize]), false);
    }

    pub fn ucomisd(&mut self, a: Xmm, b: Xmm) {
        self.op(Some(0x66), false, &[0x0F, 0x2E], a as u8, Rm::R(Reg::TODOS[b as usize]), false);
    }

    /// Inteiro de 64 bits → double.
    pub fn cvtsi2sd(&mut self, dst: Xmm, src: Reg) {
        self.op(Some(0xF2), true, &[0x0F, 0x2A], dst as u8, Rm::R(src), false);
    }

    /// Double → inteiro de 64 bits, truncando.
    pub fn cvttsd2si(&mut self, dst: Reg, src: Xmm) {
        self.op(Some(0xF2), true, &[0x0F, 0x2C], dst.n(), Rm::R(Reg::TODOS[src as usize]), false);
    }

    // ---------------------------------------------------------------- enchimento

    pub fn int3(&mut self) {
        self.b(0xCC);
    }

    /// `n` bytes de nop, nas formas de vários bytes que o manual da Intel recomenda.
    pub fn nop(&mut self, mut n: usize) {
        const NOPS: [&[u8]; 9] = [
            &[0x90],
            &[0x66, 0x90],
            &[0x0F, 0x1F, 0x00],
            &[0x0F, 0x1F, 0x40, 0x00],
            &[0x0F, 0x1F, 0x44, 0x00, 0x00],
            &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
            &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
            &[0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
            &[0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
        ];
        while n > 0 {
            let k = n.min(9);
            self.codigo.extend_from_slice(NOPS[k - 1]);
            n -= k;
        }
    }

    /// Completa com nop até a posição ser múltipla de `a` (potência de 2).
    pub fn alinhar(&mut self, a: usize) {
        debug_assert!(a.is_power_of_two());
        self.nop(self.codigo.len().wrapping_neg() & (a - 1));
    }
}
