//! Codificação conferida contra o `objdump` (o decodificador de referência): cada instrução é
//! montada, desmontada pelo objdump e comparada com o texto esperado. Sem objdump, cai num
//! subconjunto de bytes conhecidos do manual da Intel.

use falcao_asm::*;
use std::process::Command;

const R64: [&str; 16] = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
const R32: [&str; 16] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d"];
const R16: [&str; 16] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "r8w", "r9w", "r10w", "r11w", "r12w", "r13w", "r14w", "r15w"];
const R8: [&str; 16] = ["al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b", "r15b"];
const CC: [&str; 16] = ["o", "no", "b", "ae", "e", "ne", "be", "a", "s", "ns", "p", "np", "l", "ge", "le", "g"];

fn r(t: Tam, x: Reg) -> &'static str {
    if t == Tam::B64 {
        R64[x as usize]
    } else {
        R32[x as usize]
    }
}

fn xmm(x: Xmm) -> String {
    format!("xmm{}", x as usize)
}

fn hex(v: i64, t: Tam) -> String {
    match t {
        Tam::B64 => format!("0x{:x}", v as u64),
        Tam::B32 => format!("0x{:x}", v as u32),
    }
}

fn mem(m: Mem, ptr: &str) -> String {
    let mut s = String::from(R64[m.base as usize]);
    if let Some((i, e)) = m.indice {
        s += &format!("+{}*{}", R64[i as usize], [1, 2, 4, 8][e as usize]);
    }
    // O objdump mostra o deslocamento sempre que ele foi codificado (rbp/r13 levam disp8 0).
    let codificado = m.desl != 0 || matches!(m.base, Reg::Rbp | Reg::R13);
    if codificado {
        if m.desl < 0 {
            s += &format!("-0x{:x}", -(m.desl as i64));
        } else {
            s += &format!("+0x{:x}", m.desl);
        }
    }
    if ptr.is_empty() {
        format!("[{s}]")
    } else {
        format!("{ptr} PTR [{s}]")
    }
}

fn ptr(t: Tam) -> &'static str {
    if t == Tam::B64 {
        "QWORD"
    } else {
        "DWORD"
    }
}

fn alu_nome(a: Alu) -> &'static str {
    match a {
        Alu::Add => "add",
        Alu::Or => "or",
        Alu::And => "and",
        Alu::Sub => "sub",
        Alu::Xor => "xor",
        Alu::Cmp => "cmp",
    }
}

/// Uma instrução a montar e o texto que o objdump (sintaxe Intel) deve mostrar.
type Caso = (Box<dyn Fn(&mut Montador)>, String);

fn caso(f: impl Fn(&mut Montador) + 'static, t: String) -> Caso {
    (Box::new(f), t)
}

/// Gerador pseudoaleatório simples (xorshift), reprodutível.
struct Acaso(u64);

impl Acaso {
    fn n(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn ate(&mut self, k: usize) -> usize {
        (self.n() % k as u64) as usize
    }
    fn reg(&mut self) -> Reg {
        Reg::TODOS[self.ate(16)]
    }
    fn tam(&mut self) -> Tam {
        if self.n() & 1 == 0 {
            Tam::B32
        } else {
            Tam::B64
        }
    }
    fn desl(&mut self) -> i32 {
        match self.ate(4) {
            0 => 0,
            1 => self.ate(256) as i32 - 128,
            2 => self.n() as i32,
            _ => self.ate(64) as i32 * 8,
        }
    }
    fn mem(&mut self) -> Mem {
        let base = self.reg();
        let d = self.desl();
        if self.n() & 1 == 0 {
            Mem::base(base, d)
        } else {
            let mut i = self.reg();
            if i == Reg::Rsp {
                i = Reg::Rbp;
            }
            Mem::indexado(base, i, [Escala::E1, Escala::E2, Escala::E4, Escala::E8][self.ate(4)], d)
        }
    }
    fn imm(&mut self) -> i32 {
        match self.ate(3) {
            0 => self.ate(256) as i32 - 128,
            1 => self.n() as i32,
            _ => 0,
        }
    }
}

/// Todas as formas: registradores (os 16 onde se aplica), memórias, imediatos e condições.
fn casos_exaustivos() -> Vec<Caso> {
    let mut v: Vec<Caso> = Vec::new();
    let tams = [Tam::B32, Tam::B64];
    for &t in &tams {
        for &a in &Reg::TODOS {
            for &b in &Reg::TODOS {
                v.push(caso(move |m| m.mov(t, a, b), format!("mov {},{}", r(t, a), r(t, b))));
                for op in [Alu::Add, Alu::Or, Alu::And, Alu::Sub, Alu::Xor, Alu::Cmp] {
                    v.push(caso(move |m| m.alu(op, t, a, b), format!("{} {},{}", alu_nome(op), r(t, a), r(t, b))));
                }
                v.push(caso(move |m| m.test(t, a, b), format!("test {},{}", r(t, a), r(t, b))));
                v.push(caso(move |m| m.imul(t, a, b), format!("imul {},{}", r(t, a), r(t, b))));
                v.push(caso(move |m| m.cmov(Cond::L, t, a, b), format!("cmovl {},{}", r(t, a), r(t, b))));
                v.push(caso(move |m| m.movzx8(t, a, b), format!("movzx {},{}", r(t, a), R8[b as usize])));
                v.push(caso(move |m| m.movsx8(t, a, b), format!("movsx {},{}", r(t, a), R8[b as usize])));
                v.push(caso(move |m| m.movzx16(t, a, b), format!("movzx {},{}", r(t, a), R16[b as usize])));
                v.push(caso(move |m| m.movsx16(t, a, b), format!("movsx {},{}", r(t, a), R16[b as usize])));
                // Base `a` com cada índice `b` (menos rsp), e deslocamentos 0, 8 bits e 32 bits.
                for d in [0, 0x10, -0x8, 0x12345] {
                    let mb = Mem::base(a, d);
                    v.push(caso(move |m| m.carregar(t, b, mb), format!("mov {},{}", r(t, b), mem(mb, ptr(t)))));
                    v.push(caso(move |m| m.guardar(t, mb, b), format!("mov {},{}", mem(mb, ptr(t)), r(t, b))));
                    if b != Reg::Rsp {
                        let mi = Mem::indexado(a, b, Escala::E8, d);
                        v.push(caso(move |m| m.carregar(t, a, mi), format!("mov {},{}", r(t, a), mem(mi, ptr(t)))));
                    }
                }
            }
            for imm in [0, 5, -1, 127, -128, 128, 0x12345678, i32::MIN] {
                let nome = |op: Alu| alu_nome(op);
                for op in [Alu::Add, Alu::Or, Alu::And, Alu::Sub, Alu::Xor, Alu::Cmp] {
                    v.push(caso(move |m| m.alu_imm(op, t, a, imm), format!("{} {},{}", nome(op), r(t, a), hex(imm as i64, t))));
                }
                v.push(caso(move |m| m.test_imm(t, a, imm), format!("test {},{}", r(t, a), hex(imm as i64, t))));
                v.push(caso(move |m| m.imul_imm(t, a, Reg::Rbx, imm), format!("imul {},{},{}", r(t, a), r(t, Reg::Rbx), hex(imm as i64, t))));
                let mm = Mem::base(a, 8);
                v.push(caso(move |m| m.guardar_imm(t, mm, imm), format!("mov {},{}", mem(mm, ptr(t)), hex(imm as i64, t))));
            }
            for (op, nome) in [(Unario::Not, "not"), (Unario::Neg, "neg"), (Unario::Inc, "inc"), (Unario::Dec, "dec"), (Unario::Div, "div"), (Unario::Idiv, "idiv")] {
                v.push(caso(move |m| m.unario(op, t, a), format!("{nome} {}", r(t, a))));
            }
            for (op, nome) in [(Desloc::Shl, "shl"), (Desloc::Shr, "shr"), (Desloc::Sar, "sar")] {
                v.push(caso(move |m| m.desloc(op, t, a, 3), format!("{nome} {},0x3", r(t, a))));
                v.push(caso(move |m| m.desloc_cl(op, t, a), format!("{nome} {},cl", r(t, a))));
            }
            let mm = Mem::base(Reg::Rbx, 4);
            v.push(caso(move |m| m.alu_mem(Alu::Add, t, a, mm), format!("add {},{}", r(t, a), mem(mm, ptr(t)))));
            v.push(caso(move |m| m.movzx8(t, a, mm), format!("movzx {},{}", r(t, a), mem(mm, "BYTE"))));
            v.push(caso(move |m| m.movsx16(t, a, mm), format!("movsx {},{}", r(t, a), mem(mm, "WORD"))));
        }
    }
    for &a in &Reg::TODOS {
        for imm in [0i64, 1, -1, 0x7fff_ffff, -0x8000_0000] {
            v.push(caso(move |m| m.mov_imm(Tam::B64, a, imm), format!("mov {},{}", R64[a as usize], hex(imm, Tam::B64))));
            v.push(caso(move |m| m.mov_imm(Tam::B32, a, imm), format!("mov {},{}", R32[a as usize], hex(imm, Tam::B32))));
        }
        for imm in [0x1_0000_0000i64, -0x1_0000_0001, 0x1234_5678_9abc_def0] {
            v.push(caso(move |m| m.mov_imm(Tam::B64, a, imm), format!("movabs {},{}", R64[a as usize], hex(imm, Tam::B64))));
        }
        for &b in &Reg::TODOS {
            v.push(caso(move |m| m.movsxd(a, b), format!("movsxd {},{}", R64[a as usize], R32[b as usize])));
            for d in [0, 0x40, -0x200] {
                let mb = Mem::base(b, d);
                v.push(caso(move |m| m.lea(a, mb), format!("lea {},{}", R64[a as usize], mem(mb, ""))));
            }
        }
        v.push(caso(move |m| m.push(a), format!("push {}", R64[a as usize])));
        v.push(caso(move |m| m.pop(a), format!("pop {}", R64[a as usize])));
        v.push(caso(move |m| m.call(a), format!("call {}", R64[a as usize])));
        v.push(caso(move |m| m.jmp_reg(a), format!("jmp {}", R64[a as usize])));
        for (i, &c) in Cond::TODAS.iter().enumerate() {
            v.push(caso(move |m| m.setcc(c, a), format!("set{} {}", CC[i], R8[a as usize])));
            v.push(caso(move |m| m.cmov(c, Tam::B64, a, Reg::R9), format!("cmov{} {},r9", CC[i], R64[a as usize])));
        }
    }
    for &x in &Xmm::TODOS {
        for &y in &Xmm::TODOS {
            v.push(caso(move |m| m.movsd(x, y), format!("movsd {},{}", xmm(x), xmm(y))));
            for (op, nome) in [(Sse::Add, "addsd"), (Sse::Sub, "subsd"), (Sse::Mul, "mulsd"), (Sse::Div, "divsd")] {
                v.push(caso(move |m| m.sse(op, x, y), format!("{nome} {},{}", xmm(x), xmm(y))));
            }
            v.push(caso(move |m| m.ucomisd(x, y), format!("ucomisd {},{}", xmm(x), xmm(y))));
        }
        for &a in &Reg::TODOS {
            v.push(caso(move |m| m.movq_para_xmm(x, a), format!("movq {},{}", xmm(x), R64[a as usize])));
            v.push(caso(move |m| m.movq_de_xmm(a, x), format!("movq {},{}", R64[a as usize], xmm(x))));
            v.push(caso(move |m| m.cvtsi2sd(x, a), format!("cvtsi2sd {},{}", xmm(x), R64[a as usize])));
            v.push(caso(move |m| m.cvttsd2si(a, x), format!("cvttsd2si {},{}", R64[a as usize], xmm(x))));
            let mm = Mem::base(a, -16);
            v.push(caso(move |m| m.movsd_carregar(x, mm), format!("movsd {},{}", xmm(x), mem(mm, "QWORD"))));
            v.push(caso(move |m| m.movsd_guardar(mm, x), format!("movsd {},{}", mem(mm, "QWORD"), xmm(x))));
        }
    }
    v.push(caso(|m| m.ret(), "ret".into()));
    v.push(caso(|m| m.int3(), "int3".into()));
    v
}

/// Sequências aleatórias das formas com memória e imediatos, que têm mais casos especiais.
fn casos_aleatorios(n: usize, semente: u64) -> Vec<Caso> {
    let mut a = Acaso(semente);
    let mut v: Vec<Caso> = Vec::new();
    for _ in 0..n {
        let (t, x, y, mm, imm) = (a.tam(), a.reg(), a.reg(), a.mem(), a.imm());
        let c = match a.ate(9) {
            0 => caso(move |m| m.carregar(t, x, mm), format!("mov {},{}", r(t, x), mem(mm, ptr(t)))),
            1 => caso(move |m| m.guardar(t, mm, x), format!("mov {},{}", mem(mm, ptr(t)), r(t, x))),
            2 => caso(move |m| m.guardar_imm(t, mm, imm), format!("mov {},{}", mem(mm, ptr(t)), hex(imm as i64, t))),
            3 => caso(move |m| m.lea(x, mm), format!("lea {},{}", R64[x as usize], mem(mm, ""))),
            4 => caso(move |m| m.alu_mem(Alu::Sub, t, x, mm), format!("sub {},{}", r(t, x), mem(mm, ptr(t)))),
            5 => caso(move |m| m.alu_imm(Alu::Xor, t, x, imm), format!("xor {},{}", r(t, x), hex(imm as i64, t))),
            6 => caso(move |m| m.movzx8(t, x, mm), format!("movzx {},{}", r(t, x), mem(mm, "BYTE"))),
            7 => caso(move |m| m.movsxd(x, mm), format!("movsxd {},{}", R64[x as usize], mem(mm, "DWORD"))),
            _ => {
                let xm = Xmm::TODOS[y as usize];
                caso(move |m| m.movsd_carregar(xm, mm), format!("movsd {},{}", xmm(xm), mem(mm, "QWORD")))
            }
        };
        v.push(c);
    }
    v
}

fn tem_objdump() -> bool {
    Command::new("objdump").arg("--version").output().is_ok()
}

/// Desmonta `bytes` e devolve "mnemônico operandos", um por instrução.
fn objdump(bytes: &[u8], nome: &str) -> Vec<String> {
    let arq = std::env::temp_dir().join(format!("falcao_asm_{}_{nome}.bin", std::process::id()));
    std::fs::write(&arq, bytes).unwrap();
    let out = Command::new("objdump")
        .args(["-D", "-b", "binary", "-mi386:x86-64", "-M", "intel", "--insn-width=16"])
        .arg(&arq)
        .output()
        .unwrap();
    std::fs::remove_file(&arq).ok();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let partes: Vec<&str> = l.split('\t').collect();
            (partes.len() >= 3 && partes[0].trim_end().ends_with(':')).then(|| {
                let instr = partes[2].split('#').next().unwrap().trim();
                let mut it = instr.splitn(2, char::is_whitespace);
                let mnem = it.next().unwrap_or("");
                let ops = it.next().unwrap_or("").trim();
                if ops.is_empty() {
                    mnem.to_string()
                } else {
                    format!("{mnem} {ops}")
                }
            })
        })
        .collect()
}

/// Monta todos os casos num buffer só, desmonta e compara instrução a instrução.
fn conferir(casos: &[Caso], nome: &str) -> usize {
    let mut m = Montador::novo();
    for (f, _) in casos {
        f(&mut m);
    }
    let bytes = m.finalizar().unwrap();
    let vistos = objdump(&bytes, nome);
    let mut erros = Vec::new();
    for (i, (_, esperado)) in casos.iter().enumerate() {
        let visto = vistos.get(i).map(String::as_str).unwrap_or("<faltou>");
        if visto != esperado {
            erros.push(format!("#{i}: esperado `{esperado}`, objdump `{visto}`"));
        }
    }
    assert!(erros.is_empty(), "{} divergências (de {}):\n{}", erros.len(), casos.len(), erros[..erros.len().min(30)].join("\n"));
    assert_eq!(vistos.len(), casos.len(), "o objdump viu outro número de instruções");
    casos.len()
}

#[test]
fn todas_as_formas_batem_com_o_objdump() {
    if !tem_objdump() {
        eprintln!("objdump ausente: só os bytes conhecidos (teste bytes_do_manual) conferem");
        return;
    }
    let n = conferir(&casos_exaustivos(), "exaustivo");
    eprintln!("{n} instruções conferidas com o objdump");
}

#[test]
fn sequencias_aleatorias_batem_com_o_objdump() {
    if !tem_objdump() {
        return;
    }
    let mut total = 0;
    for semente in [0x9E37_79B9_7F4A_7C15u64, 1, 0xDEAD_BEEF] {
        total += conferir(&casos_aleatorios(4000, semente), &format!("acaso{semente}"));
    }
    eprintln!("{total} instruções aleatórias conferidas com o objdump");
}

/// Sem objdump: um subconjunto com bytes tirados do manual da Intel.
#[test]
fn bytes_do_manual() {
    type Bytes = (Box<dyn Fn(&mut Montador)>, &'static [u8]);
    let casos: Vec<Bytes> = vec![
        (Box::new(|m| m.mov(Tam::B64, Reg::Rax, Reg::Rbx)), &[0x48, 0x89, 0xD8]),
        (Box::new(|m| m.mov(Tam::B32, Reg::R8, Reg::Rcx)), &[0x41, 0x89, 0xC8]),
        (Box::new(|m| m.carregar(Tam::B64, Reg::Rax, Mem::base(Reg::Rsp, 0))), &[0x48, 0x8B, 0x04, 0x24]),
        (Box::new(|m| m.carregar(Tam::B64, Reg::Rax, Mem::base(Reg::R12, 0))), &[0x49, 0x8B, 0x04, 0x24]),
        (Box::new(|m| m.carregar(Tam::B64, Reg::Rax, Mem::base(Reg::Rbp, 0))), &[0x48, 0x8B, 0x45, 0x00]),
        (Box::new(|m| m.carregar(Tam::B64, Reg::Rax, Mem::base(Reg::R13, 0))), &[0x49, 0x8B, 0x45, 0x00]),
        (Box::new(|m| m.carregar(Tam::B32, Reg::Rdx, Mem::indexado(Reg::Rax, Reg::R9, Escala::E4, 0x100))), &[0x42, 0x8B, 0x94, 0x88, 0x00, 0x01, 0x00, 0x00]),
        (Box::new(|m| m.alu_imm(Alu::Add, Tam::B64, Reg::Rsp, 8)), &[0x48, 0x83, 0xC4, 0x08]),
        (Box::new(|m| m.mov_imm(Tam::B64, Reg::R10, 0x1122_3344_5566_7788)), &[0x49, 0xBA, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]),
        (Box::new(|m| m.setcc(Cond::E, Reg::Rsi)), &[0x40, 0x0F, 0x94, 0xC6]),
        (Box::new(|m| m.push(Reg::R15)), &[0x41, 0x57]),
        (Box::new(|m| m.addsd_teste()), &[0xF2, 0x41, 0x0F, 0x58, 0xC1]),
        (Box::new(|m| m.ret()), &[0xC3]),
    ];
    for (i, (f, esperado)) in casos.iter().enumerate() {
        let mut m = Montador::novo();
        f(&mut m);
        assert_eq!(m.finalizar().unwrap(), *esperado, "caso {i}");
    }
}

trait AddsdTeste {
    fn addsd_teste(&mut self);
}

impl AddsdTeste for Montador {
    fn addsd_teste(&mut self) {
        self.sse(Sse::Add, Xmm::X0, Xmm::X9);
    }
}

#[test]
fn nops_de_todos_os_tamanhos_decodificam() {
    for n in 1..=20 {
        let mut m = Montador::novo();
        m.nop(n);
        let b = m.finalizar().unwrap();
        assert_eq!(b.len(), n);
        if tem_objdump() {
            assert!(objdump(&b, &format!("nop{n}")).iter().all(|l| l.starts_with("nop") || l.starts_with("xchg ax,ax")), "nop({n})");
        }
    }
    let mut m = Montador::novo();
    m.ret();
    m.alinhar(16);
    assert_eq!(m.pos(), 16);
}

#[test]
fn rotulos_curtos_longos_e_sem_lugar() {
    // Para trás e perto: rel8.
    let mut m = Montador::novo();
    let volta = m.rotulo();
    m.ligar(volta);
    m.nop(3);
    m.jcc(Cond::Ne, volta);
    assert_eq!(m.finalizar().unwrap()[3..], [0x75, 0xFB]);
    // Para frente: sempre rel32, preenchido no fim.
    let mut m = Montador::novo();
    let fim = m.rotulo();
    m.jmp(fim);
    m.nop(2);
    m.ligar(fim);
    assert_eq!(m.finalizar().unwrap(), [0xE9, 2, 0, 0, 0, 0x66, 0x90]);
    // Para trás e longe: rel32.
    let mut m = Montador::novo();
    let l = m.rotulo();
    m.ligar(l);
    m.nop(200);
    m.jmp(l);
    let b = m.finalizar().unwrap();
    assert_eq!(b[200..], [0xE9, 0x33, 0xFF, 0xFF, 0xFF]);
    // Nunca ligado, ou ligado duas vezes: erro, não salto para lixo.
    let mut m = Montador::novo();
    let l = m.rotulo();
    m.jmp(l);
    assert_eq!(m.finalizar(), Err(Erro::RotuloSemLugar(0)));
    let mut m = Montador::novo();
    let l = m.rotulo();
    m.ligar(l);
    m.ligar(l);
    assert_eq!(m.finalizar(), Err(Erro::RotuloLigadoDuasVezes(0)));
}
