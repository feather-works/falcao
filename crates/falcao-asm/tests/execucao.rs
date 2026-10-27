//! Código montado rodando de verdade na memória executável, e a conferência do W^X.

use falcao_asm::*;

fn montar(f: impl FnOnce(&mut Montador)) -> CodigoExecutavel {
    let mut m = Montador::novo();
    f(&mut m);
    CodigoExecutavel::novo(&m.finalizar().unwrap()).unwrap()
}

fn rodar2(c: &CodigoExecutavel, a: u64, b: u64) -> u64 {
    // SEGURANÇA: o código dos testes segue System V e só usa registradores e a própria pilha.
    unsafe { c.chamar2(a, b) }
}

#[test]
fn soma() {
    let c = montar(|m| {
        m.mov(Tam::B64, Reg::Rax, Reg::Rdi);
        m.alu(Alu::Add, Tam::B64, Reg::Rax, Reg::Rsi);
        m.ret();
    });
    assert_eq!(rodar2(&c, 40, 2), 42);
    assert_eq!(rodar2(&c, u64::MAX, 1), 0);
}

#[test]
fn laco_contado_com_rotulo_para_tras_e_para_frente() {
    // soma de 0..n, saindo pelo rótulo de fim (para frente).
    let c = montar(|m| {
        let (volta, fim) = (m.rotulo(), m.rotulo());
        m.alu(Alu::Xor, Tam::B32, Reg::Rax, Reg::Rax);
        m.alu(Alu::Xor, Tam::B32, Reg::Rcx, Reg::Rcx);
        m.ligar(volta);
        m.alu(Alu::Cmp, Tam::B64, Reg::Rcx, Reg::Rdi);
        m.jcc(Cond::Ge, fim);
        m.alu(Alu::Add, Tam::B64, Reg::Rax, Reg::Rcx);
        m.unario(Unario::Inc, Tam::B64, Reg::Rcx);
        m.jmp(volta);
        m.ligar(fim);
        m.ret();
    });
    assert_eq!(rodar2(&c, 100, 0), 4950);
    assert_eq!(rodar2(&c, 0, 0), 0);
}

#[test]
fn fib_iterativo_com_registradores_altos() {
    let c = montar(|m| {
        let (volta, fim) = (m.rotulo(), m.rotulo());
        m.push(Reg::R12);
        m.mov_imm(Tam::B64, Reg::R8, 0);
        m.mov_imm(Tam::B64, Reg::R9, 1);
        m.mov(Tam::B64, Reg::R12, Reg::Rdi);
        m.ligar(volta);
        m.test(Tam::B64, Reg::R12, Reg::R12);
        m.jcc(Cond::E, fim);
        m.lea(Reg::R10, Mem::indexado(Reg::R8, Reg::R9, Escala::E1, 0));
        m.mov(Tam::B64, Reg::R8, Reg::R9);
        m.mov(Tam::B64, Reg::R9, Reg::R10);
        m.unario(Unario::Dec, Tam::B64, Reg::R12);
        m.jmp(volta);
        m.ligar(fim);
        m.mov(Tam::B64, Reg::Rax, Reg::R8);
        m.pop(Reg::R12);
        m.ret();
    });
    let fib = |n: u64| (0..n).fold((0u64, 1u64), |(a, b), _| (b, a + b)).0;
    for n in [0, 1, 2, 10, 50, 90] {
        assert_eq!(rodar2(&c, n, 0), fib(n), "fib({n})");
    }
}

#[test]
fn conta_com_doubles() {
    // (a + b) * 2.5 / (a - b), com os doubles passados como bits.
    let c = montar(|m| {
        m.movq_para_xmm(Xmm::X0, Reg::Rdi);
        m.movq_para_xmm(Xmm::X9, Reg::Rsi);
        m.movsd(Xmm::X1, Xmm::X0);
        m.sse(Sse::Add, Xmm::X0, Xmm::X9);
        m.sse(Sse::Sub, Xmm::X1, Xmm::X9);
        m.mov_imm(Tam::B64, Reg::Rax, 2.5f64.to_bits() as i64);
        m.movq_para_xmm(Xmm::X15, Reg::Rax);
        m.sse(Sse::Mul, Xmm::X0, Xmm::X15);
        m.sse(Sse::Div, Xmm::X0, Xmm::X1);
        m.movq_de_xmm(Reg::Rax, Xmm::X0);
        m.ret();
    });
    let r = f64::from_bits(rodar2(&c, 7.0f64.to_bits(), 3.0f64.to_bits()));
    assert_eq!(r, (7.0 + 3.0) * 2.5 / (7.0 - 3.0));
    // Conversões: trunc(x) e o inteiro de volta como double.
    let c = montar(|m| {
        m.movq_para_xmm(Xmm::X3, Reg::Rdi);
        m.cvttsd2si(Reg::Rax, Xmm::X3);
        m.cvtsi2sd(Xmm::X4, Reg::Rax);
        m.ucomisd(Xmm::X3, Xmm::X4);
        m.setcc(Cond::E, Reg::Rdx);
        m.movzx8(Tam::B32, Reg::Rax, Reg::Rdx);
        m.ret();
    });
    assert_eq!(rodar2(&c, 3.0f64.to_bits(), 0), 1);
    assert_eq!(rodar2(&c, 3.5f64.to_bits(), 0), 0);
}

#[test]
fn todas_as_condicoes() {
    // Para cada condição: compara rdi com rsi (com sinal e sem sinal) e devolve setcc.
    let casos: [(u64, u64); 6] = [(1, 2), (2, 1), (5, 5), (u64::MAX, 1), (1, u64::MAX), (0, 0)];
    for &c in &Cond::TODAS {
        if matches!(c, Cond::P | Cond::Np) {
            continue; // paridade: conferida pelos bytes e pelo objdump
        }
        let codigo = montar(|m| {
            let (sim, fim) = (m.rotulo(), m.rotulo());
            m.alu(Alu::Cmp, Tam::B64, Reg::Rdi, Reg::Rsi);
            m.jcc(c, sim);
            m.mov_imm(Tam::B32, Reg::Rax, 0);
            m.jmp(fim);
            m.ligar(sim);
            m.mov_imm(Tam::B32, Reg::Rax, 1);
            m.ligar(fim);
            m.ret();
        });
        for &(a, b) in &casos {
            let (sa, sb) = (a as i64, b as i64);
            let (dif, of) = sa.overflowing_sub(sb);
            let esperado = match c {
                Cond::O => of,
                Cond::No => !of,
                Cond::B => a < b,
                Cond::Ae => a >= b,
                Cond::E => a == b,
                Cond::Ne => a != b,
                Cond::Be => a <= b,
                Cond::A => a > b,
                Cond::S => dif < 0,
                Cond::Ns => dif >= 0,
                Cond::L => sa < sb,
                Cond::Ge => sa >= sb,
                Cond::Le => sa <= sb,
                Cond::G => sa > sb,
                Cond::P | Cond::Np => unreachable!(),
            };
            assert_eq!(rodar2(&codigo, a, b), u64::from(esperado), "{c:?} com ({a}, {b})");
        }
    }
}

#[test]
fn contexto_por_ponteiro_e_memoria() {
    #[repr(C)]
    struct Ctx {
        entrada: [u32; 4],
        soma: u64,
    }
    // soma = Σ entrada[i] (índice escalado por 4), devolve a soma também.
    let c = montar(|m| {
        let volta = m.rotulo();
        m.alu(Alu::Xor, Tam::B32, Reg::Rax, Reg::Rax);
        m.mov_imm(Tam::B32, Reg::Rcx, 3);
        m.ligar(volta);
        m.carregar(Tam::B32, Reg::Rdx, Mem::indexado(Reg::Rdi, Reg::Rcx, Escala::E4, 0));
        m.alu(Alu::Add, Tam::B64, Reg::Rax, Reg::Rdx);
        m.unario(Unario::Dec, Tam::B64, Reg::Rcx);
        m.jcc(Cond::Ns, volta);
        m.guardar(Tam::B64, Mem::base(Reg::Rdi, 16), Reg::Rax);
        m.ret();
    });
    let mut ctx = Ctx { entrada: [1, 20, 300, 4000], soma: 0 };
    // SEGURANÇA: o código lê `entrada` e grava `soma`, ambos dentro de `Ctx`.
    let r = unsafe { c.chamar_com(&mut ctx) };
    assert_eq!((r, ctx.soma), (4321, 4321));
}

#[test]
fn paginas_ficam_so_leitura_e_execucao() {
    let c = montar(|m| m.ret());
    let ini = c.endereco();
    let mapas = std::fs::read_to_string("/proc/self/maps").unwrap();
    let linha = mapas
        .lines()
        .find(|l| {
            let faixa = l.split_whitespace().next().unwrap();
            let (a, b) = faixa.split_once('-').unwrap();
            let (a, b) = (usize::from_str_radix(a, 16).unwrap(), usize::from_str_radix(b, 16).unwrap());
            (a..b).contains(&ini)
        })
        .expect("região do código no /proc/self/maps");
    let perm = linha.split_whitespace().nth(1).unwrap();
    assert_eq!(&perm[..3], "r-x", "W^X: {linha}");
    // E nenhuma região do processo é gravável e executável ao mesmo tempo por nossa causa.
    assert!(!perm.contains('w'));
}

#[test]
fn tamanho_invalido_e_erro() {
    assert_eq!(CodigoExecutavel::novo(&[]).err(), Some(Erro::TamanhoInvalido(0)));
}
