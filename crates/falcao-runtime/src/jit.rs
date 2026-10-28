//! N2: funções quentes viram código de máquina x86-64 (`ARQUITETURA.md`, seções 10 e 18).
//!
//! O quadro é o do N1: os registradores do bytecode ficam na pilha do motor, o quadro fica em
//! `Motor::quadros`, e o código lê e grava neles direto. Então qualquer saída deixa o estado
//! pronto para o intérprete continuar na instrução em que o código parou. Cada suposição
//! (inteiro, número, forma do objeto, item presente) é uma guarda que sai para o N1 *antes* da
//! instrução que a fez, com o acumulador de antes: a instrução roda de novo no intérprete, do
//! jeito geral. Instrução que o N2 não sabe também sai para o N1, ali mesmo.
//!
//! O código entra pelo começo da função, pelo começo de um laço (OSR, no salto para trás) e
//! logo depois de uma chamada (quando o intérprete rodou a função chamada e volta). Chamada de
//! código para código passa por [`Motor::jit_chamar`], que empurra o quadro como o intérprete e
//! roda o código da função chamada aninhado. Se ela sai para o N1 no meio, todos os códigos de
//! cima desistem ([`SAIU_ABORTO`]) e o intérprete continua do quadro do topo; os de baixo já têm
//! o `pc` de volta guardado.
//!
//! O coletor nunca roda com um valor vivo só em registrador de máquina (regra 7): ele só roda
//! nas funções nativas que o código chama (entrada de chamada, lote de voltas), e ali o único
//! valor fora da pilha do motor é o acumulador, que a chamada substitui.
//!
//! Leitura de propriedade usa o que o cache do N1 viu ao compilar: a forma do objeto e o lugar do
//! campo. Forma que não é dicionário nunca muda de layout (mudar = forma nova), então conferir o
//! número da forma basta.

use crate::formas::{Classe, Forma, Tipo};
use crate::funcoes::{ConstRt, SEM_POLI};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::valor::Valor;
use falcao_asm::{Alu, CodigoExecutavel, Cond, Desloc, Escala, Mem, Montador, Reg, Rotulo, Sse, Tam, Unario, Xmm};
use falcao_bytecode::instrucoes::{ler_operando, Operando};
use falcao_bytecode::Op;
use std::collections::HashMap;

/// Funções com mais instruções que isto ficam no N1 (código pequeno, compila rápido).
const MAX_INSTRUCOES: usize = 2000;
/// Saídas por guarda antes de desistir da função (os tipos não são os que o código supõe).
pub const MAX_FALHAS: u32 = 64;
/// Voltas por entrada: o intérprete faz a contabilidade (calor, interrupção, coleta) a cada lote.
const VOLTAS_POR_ENTRADA: i32 = 4096;

/// Estado trocado com o código gerado. A ordem dos campos é a dos deslocamentos abaixo.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Contexto {
    pub regs: *mut Valor,
    pub gaiola: *const u8,
    pub pendente: *const bool,
    pub motor: *mut Motor,
    /// Entrada e saída: o acumulador.
    pub acc: u32,
    /// Entrada: onde o código começa (uma das [`CodigoJit::entradas`]). Saída: onde o
    /// intérprete continua.
    pub pc: u32,
    /// Saída: voltas feitas nesta entrada.
    pub voltas: u32,
    /// Saída: um dos `SAIU_*`.
    pub motivo: u32,
    /// A tabela de formas (`Formas::formas`) e quantas há: tipo e classe de cada forma.
    pub formas: *const Forma,
    pub n_formas: u32,
    /// O quadro (índice em `Motor::quadros`) e o começo dos registradores dele na pilha.
    pub quadro: u32,
    pub base: u32,
    /// O contexto de variáveis capturadas do quadro (`Quadro::contexto`).
    pub contexto: u32,
    /// Topo e teto do berçário ([`crate::gaiola::Gaiola::ptr_topo_teto`]).
    pub topo: *mut u32,
    pub teto: *const u32,
    /// A tabela compacta de tipo e classe por forma (`Formas::ptr_tc`).
    pub classes: *const [u8; 2],
    /// Abaixo deste `rsp` não chama direto (`Motor::jit_limite`).
    pub limite: usize,
    /// A caixa de número que o código criou e só um registrador tem (0 = nenhuma): o
    /// `(r + x) | 0` seguinte reescreve o número nela em vez de criar outra ([`Instr`] dona).
    pub caixa: u32,
}

/// A entrada de uma função na tabela da chamada direta (`Motor::jit_tabela`), lida pelo código.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct EntradaJit {
    /// O código (0 = sem código, ou função que não é comum: o caminho geral).
    pub endereco: u64,
    /// Registradores do quadro (`max(registradores, parametros)`).
    pub tam: u32,
    pub parametros: u32,
    /// O `this`: 0 seta (undefined), 1 estrita (como veio), 2 não estrita (objeto ou o global).
    pub modo_this: u32,
    pub _livre: u32,
}

const C_REGS: i32 = 0;
const C_GAIOLA: i32 = 8;
const C_PENDENTE: i32 = 16;
const C_MOTOR: i32 = 24;
const C_ACC: i32 = 32;
const C_PC: i32 = 36;
const C_VOLTAS: i32 = 40;
const C_MOTIVO: i32 = 44;
const C_FORMAS: i32 = 48;
const C_N_FORMAS: i32 = 56;
const C_CONTEXTO: i32 = 68;
const C_TOPO: i32 = 72;
const C_TETO: i32 = 80;
const C_CLASSES: i32 = 88;
const C_LIMITE: i32 = 96;
const C_CAIXA: i32 = 104;
const C_QUADRO: i32 = 60;
const C_BASE: i32 = 64;
/// O contexto da função chamada direto fica na pilha de máquina (múltiplo de 16).
const TAM_CONTEXTO: i32 = 112;

/// Onde ficam, no `Motor`, a pilha de valores, a de quadros e a tabela da chamada direta.
const OFF_PILHA: i32 = std::mem::offset_of!(Motor, pilha) as i32;
const OFF_QUADROS: i32 = std::mem::offset_of!(Motor, quadros) as i32;
const OFF_TABELA: i32 = std::mem::offset_of!(Motor, jit_tabela) as i32;
use crate::interpretador::Quadro;
use crate::pilha::{P_CAP, P_LEN, P_PTR};
const TAM_QUADRO: i32 = std::mem::size_of::<Quadro>() as i32;
const Q_PROTO: i32 = std::mem::offset_of!(Quadro, proto) as i32;
const Q_PC: i32 = std::mem::offset_of!(Quadro, pc) as i32;
const Q_BASE: i32 = std::mem::offset_of!(Quadro, base) as i32;
const Q_FIM: i32 = std::mem::offset_of!(Quadro, fim) as i32;
const Q_ARGS: i32 = std::mem::offset_of!(Quadro, args) as i32;
const Q_TOPO: i32 = std::mem::offset_of!(Quadro, topo) as i32;
const Q_THIS: i32 = std::mem::offset_of!(Quadro, this) as i32;
const Q_FUNCAO: i32 = std::mem::offset_of!(Quadro, funcao) as i32;
const Q_NT: i32 = std::mem::offset_of!(Quadro, new_target) as i32;
const Q_CONTEXTO: i32 = std::mem::offset_of!(Quadro, contexto) as i32;
const Q_CONTEXTOS: i32 = std::mem::offset_of!(Quadro, contextos) as i32;
const Q_ENTRADA: i32 = std::mem::offset_of!(Quadro, entrada) as i32;
const Q_SUSPENSO: i32 = std::mem::offset_of!(Quadro, suspenso) as i32;
// O código grava o quadro em palavras de 32 bits: `entrada` e `construindo` dividem uma palavra
// (com o enchimento), e o tamanho é múltiplo de 4.
const _: () = {
    use std::mem::offset_of;
    assert!(Q_ENTRADA % 4 == 0 && offset_of!(Quadro, construindo) as i32 == Q_ENTRADA + 1 && Q_SUSPENSO == Q_ENTRADA + 4);
    assert!(TAM_QUADRO == Q_SUSPENSO + 4 && TAM_QUADRO % 4 == 0);
    assert!(std::mem::size_of::<EntradaJit>() == 24 && std::mem::size_of::<Contexto>() <= TAM_CONTEXTO as usize);
    assert!(offset_of!(Contexto, caixa) == C_CAIXA as usize);
    assert!(offset_of!(Contexto, limite) == C_LIMITE as usize && offset_of!(Contexto, quadro) == C_QUADRO as usize && offset_of!(Contexto, base) == C_BASE as usize);
};
/// Onde ficam, no `Motor`, o objeto global e a época dos léxicos globais (lidos pelo `LeGlobal`).
const OFF_GLOBAL: i32 = (std::mem::offset_of!(Motor, intr) + std::mem::offset_of!(crate::intrinsecos::Intrinsecos, global)) as i32;
const OFF_EPOCA: i32 = std::mem::offset_of!(Motor, epoca_lexico) as i32;

// Os deslocamentos acima são os do `Contexto` (o código gerado depende deles).
const _: () = {
    use std::mem::offset_of;
    assert!(offset_of!(Contexto, motor) == C_MOTOR as usize && offset_of!(Contexto, classes) == C_CLASSES as usize);
    assert!(offset_of!(Contexto, regs) == C_REGS as usize && offset_of!(Contexto, gaiola) == C_GAIOLA as usize);
    assert!(offset_of!(Contexto, pendente) == C_PENDENTE as usize && offset_of!(Contexto, acc) == C_ACC as usize);
    assert!(offset_of!(Contexto, pc) == C_PC as usize && offset_of!(Contexto, voltas) == C_VOLTAS as usize);
    assert!(offset_of!(Contexto, motivo) == C_MOTIVO as usize && offset_of!(Contexto, formas) == C_FORMAS as usize);
    assert!(offset_of!(Contexto, n_formas) == C_N_FORMAS as usize && offset_of!(Contexto, contexto) == C_CONTEXTO as usize);
    assert!(offset_of!(Contexto, topo) == C_TOPO as usize && offset_of!(Contexto, teto) == C_TETO as usize);
};

/// Uma suposição falhou: o intérprete roda a instrução em `pc` (conta para [`MAX_FALHAS`]).
pub const SAIU_GUARDA: u32 = 0;
/// Fim do lote de voltas (ou coleta pendente): `pc` é o começo do laço.
pub const SAIU_LOTE: u32 = 1;
/// O intérprete segue em `pc` (instrução que o N2 não sabe, salto para fora).
pub const SAIU_N1: u32 = 2;
/// `Retorna` em `pc`, com o valor no acumulador.
pub const SAIU_RETORNO: u32 = 3;
/// Uma chamada aninhada saiu para o N1: o quadro do topo já está pronto, nada a fazer aqui.
pub const SAIU_ABORTO: u32 = 4;

/// Só no gerador: a saída do laço promovido que grava o registrador e salta para `pc` (que está
/// fora dele).
const SALTO: u32 = u32::MAX;

/// Resultado das funções nativas chamadas pelo código: o valor, ou este bit para desistir.
const ABORTA: u64 = 1 << 32;

pub struct CodigoJit {
    codigo: CodigoExecutavel,
    /// Registradores do quadro que o código lê ou grava (o maior índice + 1).
    pub regs: u32,
    proto: usize,
    /// Onde o código pode começar, em ordem.
    pub entradas: Vec<u32>,
    /// Caches de propriedade vazios ao compilar (o acesso deles foi para o caminho geral).
    frios: Vec<u32>,
}

impl CodigoJit {
    /// Algum cache que estava vazio ao compilar já aprendeu uma forma.
    pub fn frio(&self, m: &Motor) -> bool {
        self.frios.iter().any(|&ci| m.protos[self.proto].caches[ci as usize].forma != u32::MAX)
    }

    /// O endereço do código (a entrada única dele; o `pc` no contexto escolhe onde começa).
    pub fn endereco(&self) -> usize {
        self.codigo.endereco()
    }

    pub fn entra_em(&self, pc: usize) -> bool {
        self.entradas.binary_search(&(pc as u32)).is_ok()
    }

    /// Roda o código.
    ///
    /// # Safety
    /// `ctx.regs` aponta para pelo menos [`CodigoJit::regs`] registradores do quadro
    /// `ctx.quadro`, `ctx.pc` é uma das entradas, e os outros ponteiros são os do motor dono do
    /// quadro, sem outro uso enquanto o código roda.
    pub unsafe fn rodar(&self, ctx: &mut Contexto) {
        // SEGURANÇA: o código foi gerado por `compilar` com a convenção `extern "C" fn(*mut
        // Contexto)`, preserva rbx/r12-r15 e só acessa os ponteiros do contexto (contrato acima).
        unsafe {
            self.codigo.chamar_com(ctx);
        }
    }
}

/// Roda o código que começa em `endereco` (o de um [`CodigoJit`], pela tabela da chamada direta).
///
/// # Safety
/// O de [`CodigoJit::rodar`], e o código do endereço está vivo.
pub unsafe fn rodar_endereco(endereco: u64, ctx: &mut Contexto) {
    // SEGURANÇA: o código segue `extern "C" fn(*mut Contexto)` (contrato acima).
    unsafe {
        let f: extern "C" fn(*mut Contexto) = std::mem::transmute(endereco as usize);
        f(ctx);
    }
}

/// Renova os ponteiros do contexto depois de uma nativa: a pilha, a gaiola e a tabela de formas
/// podem ter crescido e mudado de lugar.
fn renovar(ctx: &mut Contexto, m: &mut Motor) {
    ctx.regs = m.pilha[ctx.base as usize..].as_mut_ptr();
    ctx.gaiola = m.g.ptr();
    ctx.formas = m.formas.ptr();
    ctx.n_formas = m.formas.len() as u32;
    ctx.classes = m.formas.ptr_tc();
    ctx.contexto = m.quadros[ctx.quadro as usize].contexto.0;
}

/// O contexto e o motor dele.
///
/// SEGURANÇA (de quem chama): `ctx` é o contexto que `CodigoJit::rodar` recebeu, vivo durante a
/// chamada; o motor é o dono do quadro e não está em uso pelo Rust enquanto o código roda.
unsafe fn partes<'a>(ctx: *mut Contexto) -> (&'a mut Contexto, &'a mut Motor) {
    // SEGURANÇA: contrato acima.
    unsafe {
        let ctx = &mut *ctx;
        let m = &mut *ctx.motor;
        (ctx, m)
    }
}

/// Número como valor (marcado ou em caixa). Não coleta (alocar só marca a coleta pendente).
extern "C" fn caixa_f64(ctx: *mut Contexto, x: f64) -> u32 {
    // SEGURANÇA: chamada só pelo código gerado, com o contexto dele.
    let (ctx, m) = unsafe { partes(ctx) };
    let v = m.numero(x);
    renovar(ctx, m);
    v.0
}

/// `Chama rf, rt, n` em `ini` (o retorno é em `prox`).
extern "C" fn chamar(ctx: *mut Contexto, rf: u32, rt: u32, n: u32, ini: u32, prox: u32) -> u64 {
    // SEGURANÇA: chamada só pelo código gerado, com o contexto dele.
    let (ctx, m) = unsafe { partes(ctx) };
    let r = m.jit_chamar(ctx.quadro as usize, rf, rt, n, ini as usize, prox as usize);
    renovar(ctx, m);
    r.map_or(ABORTA, |v| u64::from(v.0))
}

/// A função chamada direto saiu de outro jeito: o valor, ou [`ABORTA`].
extern "C" fn continuar_direto(ctx: *mut Contexto, filho: *const Contexto) -> u64 {
    // SEGURANÇA: como em `entrar_direto`.
    let (ctx, m) = unsafe { partes(ctx) };
    let f = unsafe { *filho };
    let r = m.jit_continuar_direto(ctx.quadro as usize, f.quadro as usize, f.motivo, f.voltas, f.pc as usize, Valor(f.acc));
    renovar(ctx, m);
    r.map_or(ABORTA, |v| u64::from(v.0))
}

/// O caminho geral de uma instrução, pelo Rust, com o acumulador em `ctx.acc`.
extern "C" fn instrucao(ctx: *mut Contexto, op: u32, a: u32, b: u32, c: u32, ini: u32) -> u64 {
    // SEGURANÇA: chamada só pelo código gerado, com o contexto dele.
    let (ctx, m) = unsafe { partes(ctx) };
    let r = Op::de_byte(op as u8).and_then(|op| m.jit_instrucao(ctx.quadro as usize, op, a, b, c, ini as usize, Valor(ctx.acc)));
    renovar(ctx, m);
    r.map_or(ABORTA, |v| u64::from(v.0))
}

/// Anota no perfil de tipos do cache `c` os operandos de uma soma que saiu do caminho dos
/// inteiros (o que o intérprete faz no caminho lento dele).
extern "C" fn anotar(ctx: *mut Contexto, c: u32, a: u32, b: u32) {
    // SEGURANÇA: chamada só pelo código gerado, com o contexto dele.
    let (ctx, m) = unsafe { partes(ctx) };
    m.jit_anotar(ctx.quadro as usize, c, Valor(a), Valor(b));
}

struct Instr {
    ini: usize,
    fim: usize,
    op: Op,
    a: u32,
    b: u32,
    c: u32,
}

fn decodificar(codigo: &[u8]) -> Option<Vec<Instr>> {
    let mut v = Vec::new();
    let mut pc = 0;
    while pc < codigo.len() {
        let comeco = pc;
        let mut escala = 1;
        let mut byte = codigo[pc];
        pc += 1;
        if byte == Op::Largo as u8 || byte == Op::Largo4 as u8 {
            escala = if byte == Op::Largo as u8 { 2 } else { 4 };
            byte = *codigo.get(pc)?;
            pc += 1;
        }
        let op = Op::de_byte(byte)?;
        let mut vals = [0u32; 3];
        for (i, &t) in op.operandos().iter().enumerate() {
            if pc + if matches!(t, Operando::I | Operando::A) { 4 } else { escala } > codigo.len() {
                return None;
            }
            let (x, l) = ler_operando(codigo, pc, t, escala);
            if i < 3 {
                vals[i] = x;
            }
            pc += l;
        }
        v.push(Instr { ini: comeco, fim: pc, op, a: vals[0], b: vals[1], c: vals[2] });
        if v.len() > MAX_INSTRUCOES {
            return None;
        }
    }
    Some(v)
}

fn m_reg(r: u32) -> Mem {
    Mem::base(Reg::R12, 4 * r as i32)
}

const TRUE: i32 = Valor::TRUE.0 as i32;
const FALSE: i32 = Valor::FALSE.0 as i32;

/// Como ler a propriedade de um `LeProp`/`LePropAcc`, decidido ao compilar.
enum Leitura {
    /// `arr.length`: o campo interno 0 do array.
    LengthArray,
    /// Campo próprio na linha: forma esperada e deslocamento do campo no objeto.
    Campo(u32, i32),
    /// Campo na linha do protótipo direto (o método): forma do objeto, forma do protótipo e
    /// deslocamento do campo nele. O protótipo sai da tabela de formas (a forma do objeto o
    /// fixa), lido na hora: o coletor o move.
    Proto(u32, u32, i32),
}

/// Deslocamento do campo na linha `slot` de um objeto da forma `fo` (None = não é objeto comum
/// estável, ou o campo não está na linha).
fn campo_na_linha(m: &Motor, fo: u32, slot: u32) -> Option<i32> {
    if fo as usize >= m.formas.len() {
        return None;
    }
    let f = m.formas.get(crate::formas::FormaId(fo));
    if f.tipo != Tipo::Objeto || f.dicionario || slot >= f.inline {
        return None;
    }
    i32::try_from(12 + 4 * f.classe.internos().0 + 4 * slot).ok()
}

fn leitura(m: &Motor, proto: usize, k: u32, ci: u32) -> Option<Leitura> {
    let p = &m.protos[proto];
    let ConstRt::Nome(nome) = *p.consts.get(k as usize)? else { return None };
    let c = *p.caches.get(ci as usize)?;
    if c.forma != u32::MAX && c.poli == SEM_POLI {
        if c.dono == Valor::UNDEFINED {
            if let Some(d) = campo_na_linha(m, c.forma, c.deslocamento) {
                return Some(Leitura::Campo(c.forma, d));
            }
        } else if (c.forma as usize) < m.formas.len() && m.formas.get(crate::formas::FormaId(c.forma)).proto == c.dono {
            if let Some(d) = campo_na_linha(m, c.forma_dono, c.deslocamento) {
                return Some(Leitura::Proto(c.forma, c.forma_dono, d));
            }
        }
    }
    (nome == fixos::LENGTH).then_some(Leitura::LengthArray)
}

/// Onde está o campo `slot` de um objeto da forma `fo`: na linha, ou na lista de extras (o
/// deslocamento dentro dela).
enum Lugar {
    Linha(i32),
    Extra(i32),
}

fn lugar_do_campo(m: &Motor, fo: u32, slot: u32) -> Option<Lugar> {
    if let Some(d) = campo_na_linha(m, fo, slot) {
        return Some(Lugar::Linha(d));
    }
    if fo as usize >= m.formas.len() {
        return None;
    }
    let f = m.formas.get(crate::formas::FormaId(fo));
    // A forma fixa quantos campos há; os extras estão todos na lista (`ler_campo`).
    if f.tipo != Tipo::Objeto || f.dicionario || slot < f.inline || slot >= f.n {
        return None;
    }
    i32::try_from(8 + 4 * (slot - f.inline)).ok().map(Lugar::Extra)
}

/// Gravação de propriedade própria que já existe, pelo cache: forma e deslocamento do campo.
fn gravacao(m: &Motor, proto: usize, ci: u32) -> Option<(u32, i32)> {
    let c = *m.protos[proto].caches.get(ci as usize)?;
    if c.forma == u32::MAX || c.poli != SEM_POLI || c.dono != Valor::UNDEFINED {
        return None;
    }
    Some((c.forma, campo_na_linha(m, c.forma, c.deslocamento)?))
}

/// O N2 sabe a instrução (senão, ela sai para o N1).
fn sabe(ins: &Instr) -> bool {
    match ins.op {
        Op::CarregaInt => Valor::int_se_cabe(i64::from(ins.a as i32)).is_some(),
        Op::SomaInt => Valor::int_se_cabe(i64::from(ins.a as i32)).is_some_and(|_| (ins.a as i32).unsigned_abs() < 1 << 29),
        _ => SABIDAS.contains(&ins.op),
    }
}

const SABIDAS: &[Op] = &[
    Op::CarregaVerdadeiro, Op::CarregaFalso, Op::CarregaUndef, Op::CarregaBuraco, Op::CarregaConst, Op::Carrega, Op::Guarda, Op::Move,
    Op::Menor, Op::Maior, Op::MenorIgual, Op::MaiorIgual, Op::Igual, Op::Diferente, Op::Identico, Op::NaoIdentico,
    Op::SaltaSeFalso, Op::SaltaSeVerdadeiro, Op::Salta, Op::SaltaAtras, Op::Nao,
    Op::IncrementaReg, Op::DecrementaReg, Op::Incrementa, Op::Decrementa, Op::ParaNumerico, Op::ParaInt32,
    Op::Soma, Op::Sub, Op::Mul, Op::Div, Op::Resto, Op::Potencia,
    Op::E, Op::Ou, Op::OuExclusivo, Op::DeslEsq, Op::DeslDir, Op::DeslDirSemSinal,
    Op::LeProp, Op::LePropAcc, Op::GravaProp, Op::LeItem, Op::GravaItem,
    Op::CriaObjeto, Op::CriaArray, Op::DefProp, Op::Empurra, Op::CriaFuncao, Op::Typeof,
    Op::Chama, Op::Retorna, Op::LeGlobal, Op::GravaGlobal, Op::LeContexto, Op::LeContextoChecado, Op::GravaContexto,
];

/// Compila a função `proto`. None = código que o N2 não trata (gerador, async, grande demais),
/// ou a plataforma não tem memória executável.
pub fn compilar(m: &Motor, proto: usize) -> Option<CodigoJit> {
    if !cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        return None;
    }
    let fb = m.protos[proto].codigo();
    if fb.flags.gerador || fb.flags.assincrona {
        return None;
    }
    let instrs = decodificar(&fb.codigo)?;
    let sabidas: Vec<bool> = instrs.iter().map(sabe).collect();
    let regs = instrs
        .iter()
        .map(|i| match i.op {
            Op::Move => i.a.max(i.b) + 1,
            Op::Chama => i.a.max(i.b + i.c) + 1,
            Op::GravaItem => i.a.max(i.b) + 1,
            Op::Carrega
            | Op::Guarda
            | Op::Menor
            | Op::Maior
            | Op::MenorIgual
            | Op::MaiorIgual
            | Op::Igual
            | Op::Diferente
            | Op::Identico
            | Op::NaoIdentico
            | Op::IncrementaReg
            | Op::DecrementaReg
            | Op::Soma
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Resto
            | Op::Potencia
            | Op::E
            | Op::Ou
            | Op::OuExclusivo
            | Op::DeslEsq
            | Op::DeslDir
            | Op::DeslDirSemSinal
            | Op::LeProp
            | Op::GravaProp
            | Op::LeItem
            | Op::DefProp
            | Op::Empurra => i.a + 1,
            _ => 0,
        })
        .max()
        .unwrap_or(0);
    // Entradas: o começo, o começo de cada laço que roda inteiro no código (senão o OSR entraria
    // e sairia a cada volta) e a volta de cada chamada.
    let mut entradas = vec![0u32];
    for (j, ins) in instrs.iter().enumerate() {
        if ins.op == Op::SaltaAtras {
            let alvo = ins.a as usize;
            let inteiro = instrs.iter().zip(&sabidas).filter(|(x, _)| x.ini >= alvo && x.ini <= ins.ini).all(|(_, &s)| s);
            if inteiro && instrs.iter().any(|x| x.ini == alvo) {
                entradas.push(alvo as u32);
            }
        }
        if ins.op == Op::Chama && j + 1 < instrs.len() {
            entradas.push(ins.fim as u32);
        }
    }
    entradas.sort_unstable();
    entradas.dedup();

    // Registradores donos de caixa: `r = (r + x) | 0` (Soma r; ParaInt32; Guarda r), com o par
    // fundido. Qualquer outra instrução que leia ou grave um deles solta a caixa antes.
    let alvo_de_salto0 = |pc: usize| instrs.iter().any(|x| matches!(x.op, Op::Salta | Op::SaltaAtras | Op::SaltaSeFalso | Op::SaltaSeVerdadeiro) && x.a as usize == pc) || entradas.contains(&(pc as u32));
    let dono_em = |j: usize| -> Option<u32> {
        let (s, p, g) = (instrs.get(j)?, instrs.get(j + 1)?, instrs.get(j + 2)?);
        (s.op == Op::Soma && p.op == Op::ParaInt32 && g.op == Op::Guarda && g.a == s.a && !alvo_de_salto0(p.ini) && !alvo_de_salto0(g.ini)).then_some(s.a)
    };
    let donos: Vec<u32> = (0..instrs.len()).filter_map(dono_em).collect();
    let toca = |ins: &Instr, r: u32| -> bool {
        let ops = ins.op.operandos();
        let mut vals = [ins.a, ins.b, ins.c].into_iter();
        let direto = ops.iter().any(|&t| vals.next().is_some_and(|v| t == Operando::R && v == r));
        // `Chama rf, rt, n` lê também os argumentos rt+1..rt+n.
        direto || (ins.op == Op::Chama && r > ins.b && r <= ins.b + ins.c)
    };

    let promo = laco_promovido(&instrs, &sabidas, &entradas, &dono_em, &toca, &alvo_de_salto0);

    let mut a = Montador::novo();
    let rotulos: HashMap<usize, Rotulo> = instrs.iter().map(|i| (i.ini, a.rotulo())).collect();
    // Dentro do laço promovido (o registrador dele está em ebp, não na pilha do motor).
    let dentro = std::cell::Cell::new(false);
    let h_dentro = a.rotulo();
    // Saídas reunidas no fim: (rótulo, pc, motivo, grava o registrador promovido antes).
    let mut saidas: Vec<(Rotulo, u32, u32, bool)> = Vec::new();
    let mut saida = |a: &mut Montador, pc: usize, motivo: u32| -> Rotulo {
        let mat = dentro.get();
        if let Some(&(r, ..)) = saidas.iter().find(|s| s.1 == pc as u32 && s.2 == motivo && s.3 == mat) {
            return r;
        }
        let r = a.rotulo();
        saidas.push((r, pc as u32, motivo, mat));
        r
    };
    let epilogo = a.rotulo();
    let aborto = a.rotulo();

    // Prólogo: rbx = contexto, r12 = registradores, r13 = gaiola, r14d = acc, r15d = voltas,
    // ebp = o registrador do laço promovido. Seis `push` e mais 8 deixam a pilha alinhada em 16
    // para as chamadas de nativas.
    for r in [Reg::Rbx, Reg::R12, Reg::R13, Reg::R14, Reg::R15, Reg::Rbp] {
        a.push(r);
    }
    a.alu_imm(Alu::Sub, Tam::B64, Reg::Rsp, 8);
    a.mov(Tam::B64, Reg::Rbx, Reg::Rdi);
    recarregar(&mut a);
    a.carregar(Tam::B32, Reg::R14, Mem::base(Reg::Rbx, C_ACC));
    a.alu(Alu::Xor, Tam::B32, Reg::R15, Reg::R15);
    a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rbx, C_PC));
    for &e in &entradas {
        a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rax, e as i32);
        a.jcc(Cond::E, rotulos[&(e as usize)]);
    }
    // Entrada desconhecida: o intérprete segue de onde pediu.
    a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_MOTIVO), SAIU_N1 as i32);
    a.jmp(epilogo);

    // Saltos de dentro do laço promovido: o começo dele é `h_dentro` (sem recarregar o
    // registrador); para fora, passam antes por uma saída que o grava ([`SALTO`]).
    let destino = |a: &mut Montador, saida: &mut dyn FnMut(&mut Montador, usize, u32) -> Rotulo, alvo: usize| {
        if let Some((ini, j, _)) = promo.filter(|_| dentro.get()) {
            if alvo == instrs[ini].ini {
                return h_dentro;
            }
            if alvo < instrs[ini].ini || alvo > instrs[j].ini {
                return saida(a, alvo, SALTO);
            }
        }
        match rotulos.get(&alvo) {
            Some(&l) => l,
            None => saida(a, alvo, SAIU_N1),
        }
    };
    let salto = |a: &mut Montador, saida: &mut dyn FnMut(&mut Montador, usize, u32) -> Rotulo, alvo: usize, cond: Option<Cond>| {
        let l = destino(a, saida, alvo);
        match cond {
            Some(c) => a.jcc(c, l),
            None => a.jmp(l),
        }
    };
    let alvo_de_salto = |pc: usize| instrs.iter().any(|x| matches!(x.op, Op::Salta | Op::SaltaAtras | Op::SaltaSeFalso | Op::SaltaSeVerdadeiro) && x.a as usize == pc) || entradas.contains(&(pc as u32));

    // Caminhos lentos fora da linha, no fim: (lento, instrução, pronto, salto condicional fundido
    // com ela).
    let mut lentos: Vec<(Rotulo, usize, Rotulo, Option<usize>)> = Vec::new();
    // O caminho lento das somas do laço promovido: (lento, instrução, volta).
    let mut lentos_promo: Vec<(Rotulo, usize, Rotulo)> = Vec::new();
    let na_regiao = |j: usize| promo.is_some_and(|(ini, fim, _)| ini <= j && j <= fim);
    let mut i = 0;
    while i < instrs.len() {
        let ins = &instrs[i];
        a.ligar(rotulos[&ins.ini]);
        if let Some((ini, _, r)) = promo.filter(|p| p.0 == i) {
            // Começo do laço promovido, vindo de fora: o registrador para ebp, como int32.
            dentro.set(false);
            let falha = saida(&mut a, ins.ini, SAIU_GUARDA);
            carregar_int32(&mut a, r, falha);
            a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_CAIXA), 0);
            a.ligar(h_dentro);
            debug_assert_eq!(ini, i);
        }
        dentro.set(na_regiao(i));
        if !sabidas[i] {
            let l = saida(&mut a, ins.ini, SAIU_N1);
            a.jmp(l);
            i += 1;
            continue;
        }
        // Solta a caixa própria antes de quem pode guardar ou trocar o valor do registrador dono.
        let dono_aqui = dono_em(i);
        let guarda_do_dono = i >= 2 && dono_em(i - 2).is_some();
        if !donos.is_empty() && dono_aqui.is_none() && !guarda_do_dono && donos.iter().any(|&r| toca(ins, r)) {
            a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_CAIXA), 0);
        }
        // Caminho rápido em linha; o resto vai ao caminho geral (`lento`: a nativa
        // `instrucao`, que faz o que o intérprete faz). Até saltar para `lento`, o caminho rápido
        // não mudou nada.
        let lento = a.rotulo();
        let pronto = a.rotulo();
        let mut geral = true;
        match ins.op {
            Op::CarregaInt => {
                a.mov_imm(Tam::B32, Reg::R14, i64::from((ins.a as i32) << 1));
                geral = false;
            }
            Op::CarregaVerdadeiro | Op::CarregaFalso | Op::CarregaUndef | Op::CarregaBuraco => {
                let v = match ins.op {
                    Op::CarregaVerdadeiro => Valor::TRUE,
                    Op::CarregaFalso => Valor::FALSE,
                    Op::CarregaUndef => Valor::UNDEFINED,
                    _ => Valor::BURACO,
                };
                a.mov_imm(Tam::B32, Reg::R14, i64::from(v.0));
                geral = false;
            }
            Op::Carrega => {
                a.carregar(Tam::B32, Reg::R14, m_reg(ins.a));
                geral = false;
            }
            Op::CarregaConst => match m.protos[proto].consts.get(ins.a as usize) {
                // O valor lido de onde ele mora (a lista de constantes não muda de lugar nem de
                // tamanho, e o coletor atualiza o valor ali mesmo); o resto pelo Rust.
                Some(ConstRt::Valor(v)) => {
                    a.mov_imm(Tam::B64, Reg::Rax, v as *const Valor as i64);
                    a.carregar(Tam::B32, Reg::R14, Mem::base(Reg::Rax, 0));
                }
                _ => a.jmp(lento),
            },
            Op::Guarda => {
                a.guardar(Tam::B32, m_reg(ins.a), Reg::R14);
                geral = false;
            }
            Op::Move => {
                a.carregar(Tam::B32, Reg::Rax, m_reg(ins.b));
                a.guardar(Tam::B32, m_reg(ins.a), Reg::Rax);
                geral = false;
            }
            Op::Menor | Op::Maior | Op::MenorIgual | Op::MaiorIgual | Op::Igual | Op::Diferente | Op::Identico | Op::NaoIdentico => {
                // Dois inteiros marcados comparam na mesma ordem que os valores. Logo depois de
                // `CarregaInt` o acumulador já é inteiro.
                if i > 0 && instrs[i - 1].op == Op::CarregaInt && !alvo_de_salto(ins.ini) {
                    a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                    a.test_imm(Tam::B32, Reg::Rax, 1);
                    a.jcc(Cond::Ne, lento);
                } else {
                    dois_inteiros(&mut a, ins.a, lento);
                }
                let (c, nao_c) = match ins.op {
                    Op::Menor => (Cond::L, Cond::Ge),
                    Op::Maior => (Cond::G, Cond::Le),
                    Op::MenorIgual => (Cond::Le, Cond::G),
                    Op::MaiorIgual => (Cond::Ge, Cond::L),
                    Op::Igual | Op::Identico => (Cond::E, Cond::Ne),
                    _ => (Cond::Ne, Cond::E),
                };
                // Seguida de `SaltaSe…` (que ninguém salta para o meio): compara e salta, com o
                // booleano no acumulador dos dois lados (`mov` não muda as flags).
                if let Some(sj) = instrs.get(i + 1).filter(|p| matches!(p.op, Op::SaltaSeFalso | Op::SaltaSeVerdadeiro) && !alvo_de_salto(p.ini)) {
                    let falso = sj.op == Op::SaltaSeFalso;
                    a.alu(Alu::Cmp, Tam::B32, Reg::Rax, Reg::R14);
                    a.mov_imm(Tam::B32, Reg::R14, i64::from(if falso { FALSE } else { TRUE }));
                    salto(&mut a, &mut saida, sj.a as usize, Some(if falso { nao_c } else { c }));
                    a.mov_imm(Tam::B32, Reg::R14, i64::from(if falso { TRUE } else { FALSE }));
                    let segue = destino(&mut a, &mut saida, sj.fim);
                    a.ligar(rotulos[&sj.ini]);
                    lentos.push((lento, i, segue, Some(i + 1)));
                    a.jmp(segue);
                    i += 2;
                    continue;
                }
                a.mov_imm(Tam::B32, Reg::Rdx, i64::from(TRUE));
                a.alu(Alu::Cmp, Tam::B32, Reg::Rax, Reg::R14);
                a.mov_imm(Tam::B32, Reg::R14, i64::from(FALSE));
                a.cmov(c, Tam::B32, Reg::R14, Reg::Rdx);
            }
            Op::SaltaSeFalso | Op::SaltaSeVerdadeiro => {
                let alvo = destino(&mut a, &mut saida, ins.a as usize);
                salta_se(&mut a, ins, alvo, pronto, aborto);
                geral = false;
            }
            Op::Nao => {
                let outro = a.rotulo();
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R14, TRUE);
                a.jcc(Cond::Ne, outro);
                a.mov_imm(Tam::B32, Reg::R14, i64::from(FALSE));
                a.jmp(pronto);
                a.ligar(outro);
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R14, FALSE);
                a.jcc(Cond::Ne, lento);
                a.mov_imm(Tam::B32, Reg::R14, i64::from(TRUE));
            }
            Op::Salta => {
                salto(&mut a, &mut saida, ins.a as usize, None);
                geral = false;
            }
            Op::SaltaAtras => {
                // Volta: conta, e sai para a contabilidade a cada lote ou com coleta pendente.
                let destino = ins.a as usize;
                let lote = saida(&mut a, destino, SAIU_LOTE);
                a.unario(Unario::Inc, Tam::B32, Reg::R15);
                a.carregar(Tam::B64, Reg::Rax, Mem::base(Reg::Rbx, C_PENDENTE));
                a.movzx8(Tam::B32, Reg::Rax, Mem::base(Reg::Rax, 0));
                a.test(Tam::B32, Reg::Rax, Reg::Rax);
                a.jcc(Cond::Ne, lote);
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R15, VOLTAS_POR_ENTRADA);
                a.jcc(Cond::Ge, lote);
                salto(&mut a, &mut saida, destino, None);
                geral = false;
            }
            Op::IncrementaReg | Op::DecrementaReg => {
                a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                a.test_imm(Tam::B32, Reg::Rax, 1);
                a.jcc(Cond::Ne, lento);
                // ±1 marcado é ±2; estouro de 32 bits = não cabe em 31.
                a.alu_imm(if ins.op == Op::IncrementaReg { Alu::Add } else { Alu::Sub }, Tam::B32, Reg::Rax, 2);
                a.jcc(Cond::O, lento);
                a.guardar(Tam::B32, m_reg(ins.a), Reg::Rax);
            }
            Op::Incrementa | Op::Decrementa | Op::SomaInt => {
                let x = match ins.op {
                    Op::Incrementa => 2,
                    Op::Decrementa => -2,
                    _ => (ins.a as i32) << 1,
                };
                a.test_imm(Tam::B32, Reg::R14, 1);
                a.jcc(Cond::Ne, lento);
                a.mov(Tam::B32, Reg::Rax, Reg::R14);
                a.alu_imm(Alu::Add, Tam::B32, Reg::Rax, x);
                a.jcc(Cond::O, lento);
                a.mov(Tam::B32, Reg::R14, Reg::Rax);
            }
            Op::ParaNumerico | Op::ParaInt32 => {
                // Inteiro marcado já é número e int32.
                a.test_imm(Tam::B32, Reg::R14, 1);
                a.jcc(Cond::Ne, lento);
            }
            Op::Sub => {
                dois_inteiros(&mut a, ins.a, lento);
                a.alu(Alu::Sub, Tam::B32, Reg::Rax, Reg::R14);
                a.jcc(Cond::O, lento);
                a.mov(Tam::B32, Reg::R14, Reg::Rax);
            }
            Op::Mul => {
                dois_inteiros(&mut a, ins.a, lento);
                // a * (b marcado) = (a * b) marcado; zero com um negativo é -0 (double).
                a.mov(Tam::B32, Reg::Rdx, Reg::Rax);
                a.desloc(Desloc::Sar, Tam::B32, Reg::Rdx, 1);
                a.imul(Tam::B32, Reg::Rdx, Reg::R14);
                a.jcc(Cond::O, lento);
                let ok = a.rotulo();
                a.test(Tam::B32, Reg::Rdx, Reg::Rdx);
                a.jcc(Cond::Ne, ok);
                a.alu(Alu::Or, Tam::B32, Reg::Rcx, Reg::Rcx);
                a.jcc(Cond::S, lento);
                a.ligar(ok);
                a.mov(Tam::B32, Reg::R14, Reg::Rdx);
            }
            Op::E | Op::Ou | Op::OuExclusivo => {
                // Nos marcados (x << 1), and/or/xor dão o resultado já marcado.
                dois_inteiros(&mut a, ins.a, lento);
                let op = match ins.op {
                    Op::E => Alu::And,
                    Op::Ou => Alu::Or,
                    _ => Alu::Xor,
                };
                a.alu(op, Tam::B32, Reg::R14, Reg::Rax);
            }
            Op::Resto => {
                // a % b com a ≥ 0 e b > 0 (negativo pode dar -0; zero é NaN): divisão sem sinal.
                dois_inteiros(&mut a, ins.a, lento);
                a.test(Tam::B32, Reg::Rax, Reg::Rax);
                a.jcc(Cond::S, lento);
                a.mov(Tam::B32, Reg::Rcx, Reg::R14);
                a.desloc(Desloc::Sar, Tam::B32, Reg::Rcx, 1);
                a.test(Tam::B32, Reg::Rcx, Reg::Rcx);
                a.jcc(Cond::Le, lento);
                a.desloc(Desloc::Shr, Tam::B32, Reg::Rax, 1);
                a.alu(Alu::Xor, Tam::B32, Reg::Rdx, Reg::Rdx);
                a.unario(Unario::Div, Tam::B32, Reg::Rcx);
                a.alu(Alu::Add, Tam::B32, Reg::Rdx, Reg::Rdx);
                a.mov(Tam::B32, Reg::R14, Reg::Rdx);
            }
            Op::Div | Op::Potencia | Op::DeslEsq | Op::DeslDir | Op::DeslDirSemSinal => a.jmp(lento),
            Op::Soma if dentro.get() && promo.is_some_and(|p| p.2 == ins.a) && dono_em(i).is_some() => {
                // `r = (r + x) | 0` com r em ebp: a soma de 32 bits já é o ToInt32; o `Guarda`
                // não faz nada (o acumulador está morto depois, `laco_promovido`).
                let lento_p = a.rotulo();
                let volta = a.rotulo();
                a.test_imm(Tam::B32, Reg::R14, 1);
                a.jcc(Cond::Ne, lento_p);
                a.mov(Tam::B32, Reg::Rax, Reg::R14);
                a.desloc(Desloc::Sar, Tam::B32, Reg::Rax, 1);
                a.alu(Alu::Add, Tam::B32, Reg::Rbp, Reg::Rax);
                a.ligar(volta);
                a.ligar(rotulos[&instrs[i + 1].ini]);
                a.ligar(rotulos[&instrs[i + 2].ini]);
                lentos_promo.push((lento_p, i, volta));
                a.ligar(lento);
                a.ligar(pronto);
                i += 3;
                continue;
            }
            Op::Soma if instrs.get(i + 1).is_some_and(|p| p.op == Op::ParaInt32 && !alvo_de_salto(p.ini)) => {
                // `(a + b) | 0` (ninguém salta para o meio do par).
                let dobro = a.rotulo();
                let fim = a.rotulo();
                a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                a.mov(Tam::B32, Reg::Rcx, Reg::Rax);
                a.alu(Alu::Or, Tam::B32, Reg::Rcx, Reg::R14);
                a.test_imm(Tam::B32, Reg::Rcx, 1);
                a.jcc(Cond::Ne, dobro);
                // Dois marcados: a soma deles é a soma marcada, se cabe em 31 bits.
                let largo = a.rotulo();
                a.mov(Tam::B32, Reg::Rdx, Reg::Rax);
                a.alu(Alu::Add, Tam::B32, Reg::Rdx, Reg::R14);
                a.jcc(Cond::O, largo);
                a.mov(Tam::B32, Reg::R14, Reg::Rdx);
                a.jmp(pronto);
                // Dois de 31 bits: a soma cabe em 32, o resultado já é o int32.
                a.ligar(largo);
                a.mov(Tam::B32, Reg::Rsi, Reg::Rax);
                a.desloc(Desloc::Sar, Tam::B32, Reg::Rsi, 1);
                a.mov(Tam::B32, Reg::Rdx, Reg::R14);
                a.desloc(Desloc::Sar, Tam::B32, Reg::Rdx, 1);
                a.alu(Alu::Add, Tam::B32, Reg::Rsi, Reg::Rdx);
                a.jmp(fim);
                // Algum é double: soma em double e ToInt32 (os 32 bits de baixo do inteiro).
                a.ligar(dobro);
                anotar_soma(&mut a, m, proto, ins);
                // O registrador é a caixa de um int32 e o acumulador é inteiro (o `s = (s + i) | 0`
                // que passou de 31 bits): a soma exata, em inteiro, com a volta de 32 bits.
                let geral_dobro = a.rotulo();
                a.test_imm(Tam::B32, Reg::R14, 1);
                a.jcc(Cond::Ne, geral_dobro);
                a.mov(Tam::B32, Reg::Rcx, Reg::Rax);
                a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
                a.carregar(Tam::B32, Reg::R8, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0));
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, crate::formas::NUMERO.0 as i32);
                a.jcc(Cond::Ne, geral_dobro);
                a.movsd_carregar(Xmm::X0, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 8));
                a.cvttsd2si(Reg::Rsi, Xmm::X0);
                a.cvtsi2sd(Xmm::X1, Reg::Rsi);
                a.ucomisd(Xmm::X0, Xmm::X1);
                a.jcc(Cond::Ne, geral_dobro);
                a.jcc(Cond::P, geral_dobro);
                a.movsxd(Reg::Rcx, Reg::Rsi);
                a.alu(Alu::Cmp, Tam::B64, Reg::Rcx, Reg::Rsi);
                a.jcc(Cond::Ne, geral_dobro);
                a.mov(Tam::B32, Reg::Rdx, Reg::R14);
                a.desloc(Desloc::Sar, Tam::B32, Reg::Rdx, 1);
                a.alu(Alu::Add, Tam::B32, Reg::Rsi, Reg::Rdx);
                a.jmp(fim);
                a.ligar(geral_dobro);
                numero_para_xmm(&mut a, Reg::Rax, Xmm::X0, lento);
                numero_para_xmm(&mut a, Reg::R14, Xmm::X1, lento);
                a.sse(Sse::Add, Xmm::X0, Xmm::X1);
                a.cvttsd2si(Reg::Rsi, Xmm::X0);
                // NaN, infinito e |x| ≥ 2^63 dão 0x8000…: o caminho geral resolve.
                a.mov_imm(Tam::B64, Reg::Rcx, i64::MIN);
                a.alu(Alu::Cmp, Tam::B64, Reg::Rsi, Reg::Rcx);
                a.jcc(Cond::E, lento);
                a.ligar(fim);
                marcar_ou_caixa(&mut a, dono_em(i));
                a.jmp(pronto);
                // Texto, objeto…: as duas instruções pelo Rust.
                a.ligar(lento);
                chamar_instrucao(&mut a, ins, aborto);
                a.mov(Tam::B32, Reg::R14, Reg::Rax);
                chamar_instrucao(&mut a, &instrs[i + 1], aborto);
                a.mov(Tam::B32, Reg::R14, Reg::Rax);
                a.ligar(pronto);
                i += 2;
                continue;
            }
            Op::Soma => {
                let dobro = a.rotulo();
                a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                a.mov(Tam::B32, Reg::Rcx, Reg::Rax);
                a.alu(Alu::Or, Tam::B32, Reg::Rcx, Reg::R14);
                a.test_imm(Tam::B32, Reg::Rcx, 1);
                a.jcc(Cond::Ne, dobro);
                a.mov(Tam::B32, Reg::Rdx, Reg::Rax);
                a.alu(Alu::Add, Tam::B32, Reg::Rdx, Reg::R14);
                a.jcc(Cond::O, dobro);
                a.mov(Tam::B32, Reg::R14, Reg::Rdx);
                a.jmp(pronto);
                // Números (algum em caixa, ou estourou 31 bits): soma em double e põe em caixa.
                a.ligar(dobro);
                anotar_soma(&mut a, m, proto, ins);
                numero_para_xmm(&mut a, Reg::Rax, Xmm::X0, lento);
                numero_para_xmm(&mut a, Reg::R14, Xmm::X1, lento);
                a.sse(Sse::Add, Xmm::X0, Xmm::X1);
                numero_de_xmm0(&mut a);
            }
            Op::LeProp | Op::LePropAcc => {
                let (src, k, ci) = if ins.op == Op::LeProp { (Some(ins.a), ins.b, ins.c) } else { (None, ins.a, ins.b) };
                match src {
                    Some(r) => a.carregar(Tam::B32, Reg::Rax, m_reg(r)),
                    None => a.mov(Tam::B32, Reg::Rax, Reg::R14),
                }
                if !ler_prop(&mut a, m, proto, k, ci, lento) {
                    a.jmp(lento);
                }
            }
            Op::GravaProp => {
                // Campo próprio que já existe, valor que não é referência (sem barreira de
                // escrita); o resto pelo Rust.
                match gravacao(m, proto, ins.c) {
                    None => a.jmp(lento),
                    Some((forma, desl)) => {
                        a.test_imm(Tam::B32, Reg::R14, 1);
                        a.jcc(Cond::Ne, lento);
                        a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                        forma_de(&mut a, Reg::Rax, lento);
                        a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, forma as i32);
                        a.jcc(Cond::Ne, lento);
                        a.guardar(Tam::B32, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, desl), Reg::R14);
                    }
                }
            }
            Op::LeItem | Op::GravaItem => {
                // `o[i]` com i inteiro ≥ 0 e o item na lista de elementos (sem buraco). Gravar:
                // só array/objeto comum, item que já existe e valor inteiro (sem barreira, sem
                // mudar o tipo dos elementos). O resto (protótipo, array tipado, getter) é do Rust.
                let grava = ins.op == Op::GravaItem;
                let indice = if grava {
                    a.carregar(Tam::B32, Reg::Rsi, m_reg(ins.b));
                    a.test_imm(Tam::B32, Reg::R14, 1);
                    a.jcc(Cond::Ne, lento);
                    Reg::Rsi
                } else {
                    Reg::R14
                };
                a.test_imm(Tam::B32, indice, 1);
                a.jcc(Cond::Ne, lento);
                a.test(Tam::B32, indice, indice);
                a.jcc(Cond::S, lento);
                a.mov(Tam::B32, Reg::Rsi, indice);
                a.desloc(Desloc::Shr, Tam::B32, Reg::Rsi, 1);
                a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                forma_de(&mut a, Reg::Rax, lento);
                if grava {
                    conferir_classe(&mut a, lento, &[Classe::Array, Classe::Comum]);
                    // Array: dentro do `length`.
                    let comum = a.rotulo();
                    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, Classe::Array as i32);
                    a.jcc(Cond::Ne, comum);
                    a.alu_mem(Alu::Cmp, Tam::B32, Reg::Rsi, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 12));
                    a.jcc(Cond::Ae, lento);
                    a.ligar(comum);
                } else {
                    conferir_forma(&mut a, lento, None);
                }
                // rdx = endereço da lista de elementos.
                a.carregar(Tam::B32, Reg::Rdx, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 8));
                a.alu_imm(Alu::And, Tam::B32, Reg::Rdx, -2);
                a.lea(Reg::Rdx, Mem::indexado(Reg::R13, Reg::Rdx, Escala::E1, 0));
                a.alu_mem(Alu::Cmp, Tam::B32, Reg::Rsi, Mem::base(Reg::Rdx, 4));
                a.jcc(Cond::Ae, lento);
                a.carregar(Tam::B32, Reg::Rax, Mem::indexado(Reg::Rdx, Reg::Rsi, Escala::E4, 8));
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rax, Valor::BURACO.0 as i32);
                a.jcc(Cond::E, lento);
                if grava {
                    a.guardar(Tam::B32, Mem::indexado(Reg::Rdx, Reg::Rsi, Escala::E4, 8), Reg::R14);
                } else {
                    a.mov(Tam::B32, Reg::R14, Reg::Rax);
                }
            }
            Op::LeContexto | Op::LeContextoChecado | Op::GravaContexto => {
                // O contexto `a` níveis acima (o pai fica no +8), e a variável `b` no +12+4b.
                a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rbx, C_CONTEXTO));
                for _ in 0..ins.a {
                    a.alu_imm(Alu::And, Tam::B32, Reg::Rax, -2);
                    a.carregar(Tam::B32, Reg::Rax, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 8));
                }
                a.alu_imm(Alu::And, Tam::B32, Reg::Rax, -2);
                let campo = Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, i32::try_from(12 + 4 * u64::from(ins.b)).ok()?);
                if ins.op == Op::GravaContexto {
                    barreira(&mut a, m, Reg::Rax, lento);
                    a.guardar(Tam::B32, campo, Reg::R14);
                } else {
                    a.carregar(Tam::B32, Reg::Rcx, campo);
                    if ins.op == Op::LeContextoChecado {
                        a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rcx, Valor::BURACO.0 as i32);
                        a.jcc(Cond::E, lento);
                    }
                    a.mov(Tam::B32, Reg::R14, Reg::Rcx);
                }
            }
            Op::CriaObjeto | Op::CriaArray => {
                // Objeto novo no berçário, como `novo_objeto` (o resto vai à nativa).
                let forma = if ins.op == Op::CriaArray {
                    m.intr.forma_array
                } else if ins.a <= 4 {
                    m.intr.forma_objeto
                } else {
                    crate::formas::FormaId(u32::MAX)
                };
                if forma.0 == u32::MAX {
                    a.jmp(lento);
                } else {
                    let f = m.formas.get(forma);
                    let (n_int, mascara) = f.classe.internos();
                    let mut palavras = vec![forma.0, Valor::LISTA_VAZIA.0, Valor::LISTA_VAZIA.0];
                    palavras.extend((0..n_int).map(|i| if mascara & (1 << i) != 0 { Valor::UNDEFINED.0 } else { 0 }));
                    palavras.extend((0..f.inline).map(|_| Valor::UNDEFINED.0));
                    let bytes = (4 * palavras.len() as i32 + 7) & !7;
                    // Array literal de n itens: a lista de elementos já com n lugares (buracos),
                    // logo depois do objeto — os `Empurra` seguintes só preenchem.
                    let n = if ins.op == Op::CriaArray { ins.a.min(64) as i32 } else { 0 };
                    let lista = if n > 0 { (8 + 4 * n + 7) & !7 } else { 0 };
                    if n > 0 {
                        palavras[2] = 0; // preenchido abaixo
                    }
                    a.carregar(Tam::B64, Reg::Rdx, Mem::base(Reg::Rbx, C_TOPO));
                    a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rdx, 0));
                    a.lea(Reg::Rcx, Mem::base(Reg::Rax, bytes + lista));
                    a.carregar(Tam::B64, Reg::R8, Mem::base(Reg::Rbx, C_TETO));
                    a.alu_mem(Alu::Cmp, Tam::B32, Reg::Rcx, Mem::base(Reg::R8, 0));
                    a.jcc(Cond::A, lento);
                    a.guardar(Tam::B32, Mem::base(Reg::Rdx, 0), Reg::Rcx);
                    for (j, &w) in palavras.iter().enumerate() {
                        a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 4 * j as i32), w as i32);
                    }
                    if n > 0 {
                        // A lista em d+bytes: [LISTA][n][buraco…]; o objeto aponta para ela.
                        a.lea(Reg::Rcx, Mem::base(Reg::Rax, bytes + 1));
                        a.guardar(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 8), Reg::Rcx);
                        a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, bytes), crate::formas::LISTA.0 as i32);
                        a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, bytes + 4), n);
                        for j in 0..n {
                            a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, bytes + 8 + 4 * j), Valor::BURACO.0 as i32);
                        }
                    }
                    a.alu_imm(Alu::Or, Tam::B32, Reg::Rax, 1);
                    a.mov(Tam::B32, Reg::R14, Reg::Rax);
                }
            }
            Op::DefProp => {
                // Transição de forma do objeto literal pelo cache: forma de antes, de depois e o
                // campo novo (na linha).
                let e = m.protos[proto].caches.get(ins.c as usize).copied();
                match e.filter(|e| e.forma != u32::MAX).and_then(|e| Some((e.forma, e.forma_dono, campo_na_linha(m, e.forma_dono, e.deslocamento)?))) {
                    None => a.jmp(lento),
                    Some((antes, depois, desl)) => {
                        a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                        forma_de(&mut a, Reg::Rax, lento);
                        a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, antes as i32);
                        a.jcc(Cond::Ne, lento);
                        barreira(&mut a, m, Reg::Rcx, lento);
                        a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0), depois as i32);
                        a.guardar(Tam::B32, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, desl), Reg::R14);
                    }
                }
            }
            Op::Empurra => {
                // Item inteiro no array literal (a forma inicial de array: nem nomes nem buracos)
                // que ainda cabe na lista.
                a.test_imm(Tam::B32, Reg::R14, 1);
                a.jcc(Cond::Ne, lento);
                a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
                forma_de(&mut a, Reg::Rax, lento);
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, m.intr.forma_array.0 as i32);
                a.jcc(Cond::Ne, lento);
                // esi = length, rdx = lista.
                a.carregar(Tam::B32, Reg::Rsi, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 12));
                a.carregar(Tam::B32, Reg::Rdx, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 8));
                a.alu_imm(Alu::And, Tam::B32, Reg::Rdx, -2);
                a.lea(Reg::Rdx, Mem::indexado(Reg::R13, Reg::Rdx, Escala::E1, 0));
                a.alu_mem(Alu::Cmp, Tam::B32, Reg::Rsi, Mem::base(Reg::Rdx, 4));
                a.jcc(Cond::Ae, lento);
                a.guardar(Tam::B32, Mem::indexado(Reg::Rdx, Reg::Rsi, Escala::E4, 8), Reg::R14);
                a.unario(Unario::Inc, Tam::B32, Reg::Rsi);
                a.guardar(Tam::B32, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 12), Reg::Rsi);
            }
            Op::LeGlobal => {
                // Pelo cache: a forma do objeto global e a época dos `let`/`const` globais iguais às
                // de quando o cache encheu.
                let e = m.protos[proto].caches.get(ins.b as usize).copied();
                match e.filter(|e| e.forma != u32::MAX).and_then(|e| Some((e.forma, e.forma_dono, lugar_do_campo(m, e.forma, e.deslocamento)?))) {
                    None => a.jmp(lento),
                    Some((forma, epoca, lugar)) => {
                        a.carregar(Tam::B64, Reg::Rdx, Mem::base(Reg::Rbx, C_MOTOR));
                        a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rdx, OFF_EPOCA));
                        a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rax, epoca as i32);
                        a.jcc(Cond::Ne, lento);
                        a.carregar(Tam::B32, Reg::Rcx, Mem::base(Reg::Rdx, OFF_GLOBAL));
                        a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
                        a.carregar(Tam::B32, Reg::R8, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0));
                        a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, forma as i32);
                        a.jcc(Cond::Ne, lento);
                        match lugar {
                            Lugar::Linha(desl) => a.carregar(Tam::B32, Reg::R14, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, desl)),
                            Lugar::Extra(desl) => {
                                // A lista de campos extras (no +4) e o campo nela.
                                a.carregar(Tam::B32, Reg::Rcx, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 4));
                                a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
                                a.carregar(Tam::B32, Reg::R14, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, desl));
                            }
                        }
                    }
                }
            }
            Op::CriaFuncao | Op::Typeof | Op::GravaGlobal => {
                a.jmp(lento);
            }
            Op::Chama => {
                // Direto, sem o Rust: a função (objeto função de bytecode com código comum, na
                // tabela), o quadro dela empurrado aqui mesmo (como `empilhar`), o contexto dela
                // na pilha de máquina e `call` no código. Qualquer outra coisa: o caminho geral.
                // (A pilha está alinhada em 16 aqui; TAM_CONTEXTO também é múltiplo.)
                let caminho_geral = a.rotulo();
                let outro = a.rotulo();
                let (rf, rt, n) = (ins.a, ins.b, ins.c);
                a.alu_imm(Alu::Sub, Tam::B64, Reg::Rsp, TAM_CONTEXTO);
                // eax = f, rcx = o objeto, r10d = o código (índice em `protos`).
                a.carregar(Tam::B32, Reg::Rax, m_reg(rf));
                forma_de(&mut a, Reg::Rax, caminho_geral);
                a.alu_mem(Alu::Cmp, Tam::B32, Reg::R8, Mem::base(Reg::Rbx, C_N_FORMAS));
                a.jcc(Cond::Ae, caminho_geral);
                a.carregar(Tam::B64, Reg::R9, Mem::base(Reg::Rbx, C_CLASSES));
                a.movzx16(Tam::B32, Reg::R11, Mem::indexado(Reg::R9, Reg::R8, Escala::E2, 0));
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, Tipo::Objeto as i32 | (Classe::Funcao as i32) << 8);
                a.jcc(Cond::Ne, caminho_geral);
                a.carregar(Tam::B32, Reg::R10, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 12 + 4 * crate::funcoes::F_CODIGO as i32));
                a.test(Tam::B32, Reg::R10, Reg::R10);
                a.jcc(Cond::S, caminho_geral); // nativa (BIT_NATIVA)
                // r9 = a entrada dela na tabela, rsi = o código.
                a.carregar(Tam::B64, Reg::Rdx, Mem::base(Reg::Rbx, C_MOTOR));
                a.alu_mem(Alu::Cmp, Tam::B64, Reg::R10, Mem::base(Reg::Rdx, OFF_TABELA + P_LEN));
                a.jcc(Cond::Ae, caminho_geral);
                a.imul_imm(Tam::B64, Reg::R11, Reg::R10, std::mem::size_of::<EntradaJit>() as i32);
                a.alu_mem(Alu::Add, Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_TABELA + P_PTR));
                a.mov(Tam::B64, Reg::R9, Reg::R11);
                a.carregar(Tam::B64, Reg::Rsi, Mem::base(Reg::R9, 0));
                a.test(Tam::B64, Reg::Rsi, Reg::Rsi);
                a.jcc(Cond::E, caminho_geral);
                // Argumentos a mais que os parâmetros: o geral (o quadro é outro).
                a.mov_imm(Tam::B32, Reg::R11, i64::from(n));
                a.alu_mem(Alu::Cmp, Tam::B32, Reg::R11, Mem::base(Reg::R9, 12));
                a.jcc(Cond::A, caminho_geral);
                // Pilha de máquina, coleta pendente, quadros (o limite e a capacidade).
                a.alu_mem(Alu::Cmp, Tam::B64, Reg::Rsp, Mem::base(Reg::Rbx, C_LIMITE));
                a.jcc(Cond::B, caminho_geral);
                a.carregar(Tam::B64, Reg::R11, Mem::base(Reg::Rbx, C_PENDENTE));
                a.movzx8(Tam::B32, Reg::R11, Mem::base(Reg::R11, 0));
                a.test(Tam::B32, Reg::R11, Reg::R11);
                a.jcc(Cond::Ne, caminho_geral);
                a.carregar(Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_QUADROS + P_LEN));
                a.alu_imm(Alu::Cmp, Tam::B64, Reg::R11, 10_000);
                a.jcc(Cond::A, caminho_geral);
                a.alu_mem(Alu::Cmp, Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_QUADROS + P_CAP));
                a.jcc(Cond::Ae, caminho_geral);
                // r8 = topo da pilha de valores (a base do quadro novo), edi = tamanho dele.
                a.carregar(Tam::B64, Reg::R8, Mem::base(Reg::Rdx, OFF_PILHA + P_LEN));
                a.carregar(Tam::B32, Reg::Rdi, Mem::base(Reg::R9, 8));
                a.lea(Reg::R11, Mem::indexado(Reg::R8, Reg::Rdi, Escala::E1, 0));
                a.alu_mem(Alu::Cmp, Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_PILHA + P_CAP));
                a.jcc(Cond::A, caminho_geral);
                // O `this` (guardado no acc do contexto filho por enquanto) e o f (no pc dele).
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_PC), Reg::Rax);
                let this_ok = a.rotulo();
                let global = a.rotulo();
                let nao_seta = a.rotulo();
                a.carregar(Tam::B32, Reg::R11, Mem::base(Reg::R9, 16));
                a.test(Tam::B32, Reg::R11, Reg::R11);
                a.jcc(Cond::Ne, nao_seta);
                a.mov_imm(Tam::B32, Reg::R11, i64::from(Valor::UNDEFINED.0));
                a.jmp(this_ok);
                a.ligar(nao_seta);
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, 1);
                a.carregar(Tam::B32, Reg::R11, m_reg(rt));
                a.jcc(Cond::E, this_ok);
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, Valor::UNDEFINED.0 as i32);
                a.jcc(Cond::E, global);
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, Valor::NULL.0 as i32);
                a.jcc(Cond::E, global);
                // Não estrita com objeto: fica; primitivo vira objeto (o geral).
                a.test_imm(Tam::B32, Reg::R11, 1);
                a.jcc(Cond::E, caminho_geral);
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_ACC), Reg::R11);
                a.alu_imm(Alu::And, Tam::B32, Reg::R11, -2);
                a.carregar(Tam::B32, Reg::R11, Mem::indexado(Reg::R13, Reg::R11, Escala::E1, 0));
                a.alu_mem(Alu::Cmp, Tam::B32, Reg::R11, Mem::base(Reg::Rbx, C_N_FORMAS));
                a.jcc(Cond::Ae, caminho_geral);
                a.carregar(Tam::B64, Reg::Rcx, Mem::base(Reg::Rbx, C_CLASSES));
                a.movzx8(Tam::B32, Reg::R11, Mem::indexado(Reg::Rcx, Reg::R11, Escala::E2, 0));
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, Tipo::Objeto as i32);
                a.jcc(Cond::Ne, caminho_geral);
                a.carregar(Tam::B32, Reg::R11, Mem::base(Reg::Rsp, C_ACC));
                a.jmp(this_ok);
                a.ligar(global);
                a.carregar(Tam::B32, Reg::R11, Mem::base(Reg::Rdx, OFF_GLOBAL));
                a.ligar(this_ok);
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_ACC), Reg::R11);
                // Daqui em diante não volta mais ao geral: empurra.
                // Os argumentos e o resto dos registradores (undefined) na pilha de valores.
                a.carregar(Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_PILHA + P_PTR));
                a.lea(Reg::R11, Mem::indexado(Reg::R11, Reg::R8, Escala::E4, 0));
                for j in 0..n {
                    a.carregar(Tam::B32, Reg::Rax, m_reg(rt + 1 + j));
                    a.guardar(Tam::B32, Mem::base(Reg::R11, 4 * j as i32), Reg::Rax);
                }
                let enche = a.rotulo();
                let cheio = a.rotulo();
                a.mov_imm(Tam::B32, Reg::Rax, i64::from(n));
                a.ligar(enche);
                a.alu(Alu::Cmp, Tam::B32, Reg::Rax, Reg::Rdi);
                a.jcc(Cond::Ae, cheio);
                a.guardar_imm(Tam::B32, Mem::indexado(Reg::R11, Reg::Rax, Escala::E4, 0), Valor::UNDEFINED.0 as i32);
                a.unario(Unario::Inc, Tam::B32, Reg::Rax);
                a.jmp(enche);
                a.ligar(cheio);
                a.lea(Reg::Rax, Mem::indexado(Reg::R8, Reg::Rdi, Escala::E1, 0));
                a.guardar(Tam::B64, Mem::base(Reg::Rdx, OFF_PILHA + P_LEN), Reg::Rax);
                // O quadro novo (rax) no fim da pilha de quadros; r11 = o índice dele.
                a.carregar(Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_QUADROS + P_LEN));
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_QUADRO), Reg::R11);
                a.imul_imm(Tam::B64, Reg::Rax, Reg::R11, TAM_QUADRO);
                a.alu_mem(Alu::Add, Tam::B64, Reg::Rax, Mem::base(Reg::Rdx, OFF_QUADROS + P_PTR));
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_PROTO), Reg::R10);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rax, Q_PC), 0);
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_BASE), Reg::R8);
                a.lea(Reg::R9, Mem::indexado(Reg::R8, Reg::Rdi, Escala::E1, 0));
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_FIM), Reg::R9);
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_ARGS), Reg::R8);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rax, Q_ARGS + 4), n as i32);
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_TOPO), Reg::R8);
                a.carregar(Tam::B32, Reg::R9, Mem::base(Reg::Rsp, C_ACC));
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_THIS), Reg::R9);
                a.carregar(Tam::B32, Reg::R9, Mem::base(Reg::Rsp, C_PC));
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_FUNCAO), Reg::R9);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rax, Q_NT), Valor::UNDEFINED.0 as i32);
                // O contexto da função: o campo interno F_CONTEXTO dela (r9d, também para o filho).
                a.alu_imm(Alu::And, Tam::B32, Reg::R9, -2);
                a.carregar(Tam::B32, Reg::R9, Mem::indexado(Reg::R13, Reg::R9, Escala::E1, 12 + 4 * crate::funcoes::F_CONTEXTO as i32));
                a.guardar(Tam::B32, Mem::base(Reg::Rax, Q_CONTEXTO), Reg::R9);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rax, Q_CONTEXTOS), 0);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rax, Q_ENTRADA), 0);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rax, Q_SUSPENSO), crate::interpretador::SEM_SUSPENSO as i32);
                a.carregar(Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_QUADROS + P_LEN));
                a.unario(Unario::Inc, Tam::B64, Reg::R11);
                a.guardar(Tam::B64, Mem::base(Reg::Rdx, OFF_QUADROS + P_LEN), Reg::R11);
                // O pc de volta deste quadro (o que o intérprete faz antes de chamar).
                quadro_de(&mut a, Reg::Rcx);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rcx, Q_PC), ins.fim as i32);
                // O contexto filho: o deste, com os registradores, o quadro e o contexto dele.
                a.carregar(Tam::B64, Reg::Rcx, Mem::base(Reg::Rdx, OFF_PILHA + P_PTR));
                a.lea(Reg::Rcx, Mem::indexado(Reg::Rcx, Reg::R8, Escala::E4, 0));
                a.guardar(Tam::B64, Mem::base(Reg::Rsp, C_REGS), Reg::Rcx);
                for off in [C_GAIOLA, C_PENDENTE, C_MOTOR, C_FORMAS, C_TOPO, C_TETO, C_CLASSES, C_LIMITE] {
                    a.carregar(Tam::B64, Reg::Rcx, Mem::base(Reg::Rbx, off));
                    a.guardar(Tam::B64, Mem::base(Reg::Rsp, off), Reg::Rcx);
                }
                a.carregar(Tam::B32, Reg::Rcx, Mem::base(Reg::Rbx, C_N_FORMAS));
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_N_FORMAS), Reg::Rcx);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rsp, C_ACC), Valor::UNDEFINED.0 as i32);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rsp, C_PC), 0);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rsp, C_VOLTAS), 0);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rsp, C_MOTIVO), 0);
                a.guardar_imm(Tam::B32, Mem::base(Reg::Rsp, C_CAIXA), 0);
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_BASE), Reg::R8);
                a.guardar(Tam::B32, Mem::base(Reg::Rsp, C_CONTEXTO), Reg::R9);
                a.mov(Tam::B64, Reg::Rdi, Reg::Rsp);
                a.call(Reg::Rsi);
                // A gaiola e a tabela de formas podem ter mudado de lugar lá dentro: as do filho.
                for off in [C_GAIOLA, C_FORMAS, C_CLASSES] {
                    a.carregar(Tam::B64, Reg::Rcx, Mem::base(Reg::Rsp, off));
                    a.guardar(Tam::B64, Mem::base(Reg::Rbx, off), Reg::Rcx);
                }
                a.carregar(Tam::B32, Reg::Rcx, Mem::base(Reg::Rsp, C_N_FORMAS));
                a.guardar(Tam::B32, Mem::base(Reg::Rbx, C_N_FORMAS), Reg::Rcx);
                // Voltou com `Retorna` e sem voltas: desempilha aqui (o `Retorna` do intérprete).
                a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rsp, C_MOTIVO));
                a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rax, SAIU_RETORNO as i32);
                a.jcc(Cond::Ne, outro);
                a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rsp, C_VOLTAS));
                a.test(Tam::B32, Reg::Rax, Reg::Rax);
                a.jcc(Cond::Ne, outro);
                a.carregar(Tam::B64, Reg::Rdx, Mem::base(Reg::Rbx, C_MOTOR));
                a.carregar(Tam::B64, Reg::R11, Mem::base(Reg::Rdx, OFF_QUADROS + P_LEN));
                a.unario(Unario::Dec, Tam::B64, Reg::R11);
                a.guardar(Tam::B64, Mem::base(Reg::Rdx, OFF_QUADROS + P_LEN), Reg::R11);
                quadro_de(&mut a, Reg::Rcx);
                a.carregar(Tam::B32, Reg::Rcx, Mem::base(Reg::Rcx, Q_FIM));
                a.guardar(Tam::B64, Mem::base(Reg::Rdx, OFF_PILHA + P_LEN), Reg::Rcx);
                // A pilha de valores pode ter mudado de lugar: os registradores deste de novo.
                a.carregar(Tam::B64, Reg::Rcx, Mem::base(Reg::Rdx, OFF_PILHA + P_PTR));
                a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rbx, C_BASE));
                a.lea(Reg::R12, Mem::indexado(Reg::Rcx, Reg::Rax, Escala::E4, 0));
                a.guardar(Tam::B64, Mem::base(Reg::Rbx, C_REGS), Reg::R12);
                a.carregar(Tam::B64, Reg::R13, Mem::base(Reg::Rbx, C_GAIOLA));
                a.carregar(Tam::B32, Reg::R14, Mem::base(Reg::Rsp, C_ACC));
                a.alu_imm(Alu::Add, Tam::B64, Reg::Rsp, TAM_CONTEXTO);
                a.jmp(pronto);
                a.ligar(outro);
                a.mov(Tam::B64, Reg::Rdi, Reg::Rbx);
                a.mov(Tam::B64, Reg::Rsi, Reg::Rsp);
                a.mov_imm(Tam::B64, Reg::Rax, continuar_direto as extern "C" fn(*mut Contexto, *const Contexto) -> u64 as usize as i64);
                a.call(Reg::Rax);
                recarregar(&mut a);
                a.alu_imm(Alu::Add, Tam::B64, Reg::Rsp, TAM_CONTEXTO);
                a.mov(Tam::B64, Reg::Rcx, Reg::Rax);
                a.desloc(Desloc::Shr, Tam::B64, Reg::Rcx, 32);
                a.jcc(Cond::Ne, aborto);
                a.mov(Tam::B32, Reg::R14, Reg::Rax);
                a.jmp(pronto);
                // O caminho geral (nativa, função sem código, coleta pendente…).
                a.ligar(caminho_geral);
                a.alu_imm(Alu::Add, Tam::B64, Reg::Rsp, TAM_CONTEXTO);
                a.mov(Tam::B64, Reg::Rdi, Reg::Rbx);
                a.mov_imm(Tam::B32, Reg::Rsi, i64::from(ins.a));
                a.mov_imm(Tam::B32, Reg::Rdx, i64::from(ins.b));
                a.mov_imm(Tam::B32, Reg::Rcx, i64::from(ins.c));
                a.mov_imm(Tam::B32, Reg::R8, ins.ini as i64);
                a.mov_imm(Tam::B32, Reg::R9, ins.fim as i64);
                let f = chamar as extern "C" fn(*mut Contexto, u32, u32, u32, u32, u32) -> u64;
                resultado_de_nativa(&mut a, f as usize, aborto);
                a.mov(Tam::B32, Reg::R14, Reg::Rax);
                geral = false;
            }
            Op::Retorna => {
                let l = saida(&mut a, ins.ini, SAIU_RETORNO);
                a.jmp(l);
                geral = false;
            }
            _ => unreachable!("`sabe` aceitou {:?}", ins.op),
        }
        if geral {
            lentos.push((lento, i, pronto, None));
            a.ligar(pronto);
        } else {
            a.ligar(lento);
            a.ligar(pronto);
        }
        i += 1;
    }
    // Depois da última instrução (o compilador sempre termina com `Retorna`): não chega aqui.
    dentro.set(false);
    let fim = saida(&mut a, fb.codigo.len(), SAIU_N1);
    a.jmp(fim);
    // Os caminhos lentos de dentro do laço promovido gravam o registrador antes (a nativa e
    // quem ela chamar o leem da pilha do motor) e voltam com ele ainda em ebp (só as somas
    // promovidas o mudam).
    for (lento_p, j, volta) in lentos_promo {
        let r = promo.map_or(0, |p| p.2);
        a.ligar(lento_p);
        materializar(&mut a, r);
        chamar_instrucao(&mut a, &instrs[j], aborto);
        a.mov(Tam::B32, Reg::R14, Reg::Rax);
        chamar_instrucao(&mut a, &instrs[j + 1], aborto);
        a.mov(Tam::B32, Reg::R14, Reg::Rax);
        a.guardar(Tam::B32, m_reg(r), Reg::R14);
        // O resultado é int32 (marcado ou em caixa).
        let caixa = a.rotulo();
        a.mov(Tam::B32, Reg::Rax, Reg::R14);
        a.test_imm(Tam::B32, Reg::Rax, 1);
        a.jcc(Cond::Ne, caixa);
        a.desloc(Desloc::Sar, Tam::B32, Reg::Rax, 1);
        a.mov(Tam::B32, Reg::Rbp, Reg::Rax);
        a.jmp(volta);
        a.ligar(caixa);
        a.alu_imm(Alu::And, Tam::B32, Reg::Rax, -2);
        a.movsd_carregar(Xmm::X0, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 8));
        a.cvttsd2si(Reg::Rax, Xmm::X0);
        a.mov(Tam::B32, Reg::Rbp, Reg::Rax);
        a.jmp(volta);
    }
    for (lento, j, pronto, sj) in lentos {
        a.ligar(lento);
        if let Some(p) = promo.filter(|_| na_regiao(j)) {
            materializar(&mut a, p.2);
        }
        chamar_instrucao(&mut a, &instrs[j], aborto);
        a.mov(Tam::B32, Reg::R14, Reg::Rax);
        if let Some(k) = sj {
            let alvo = destino(&mut a, &mut saida, instrs[k].a as usize);
            salta_se(&mut a, &instrs[k], alvo, pronto, aborto);
        }
        a.jmp(pronto);
    }

    for (r, pc, motivo, mat) in saidas {
        a.ligar(r);
        if let Some(p) = promo.filter(|_| mat) {
            materializar(&mut a, p.2);
        }
        if motivo == SALTO {
            if let Some(&l) = rotulos.get(&(pc as usize)) {
                a.jmp(l);
                continue;
            }
            a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_MOTIVO), SAIU_N1 as i32);
        } else {
            a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_MOTIVO), motivo as i32);
        }
        a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_PC), pc as i32);
        a.jmp(epilogo);
    }
    a.ligar(aborto);
    a.guardar_imm(Tam::B32, Mem::base(Reg::Rbx, C_MOTIVO), SAIU_ABORTO as i32);
    a.ligar(epilogo);
    a.guardar(Tam::B32, Mem::base(Reg::Rbx, C_ACC), Reg::R14);
    a.guardar(Tam::B32, Mem::base(Reg::Rbx, C_VOLTAS), Reg::R15);
    a.alu_imm(Alu::Add, Tam::B64, Reg::Rsp, 8);
    for r in [Reg::Rbp, Reg::R15, Reg::R14, Reg::R13, Reg::R12, Reg::Rbx] {
        a.pop(r);
    }
    a.ret();
    let bytes = a.finalizar().ok()?;
    let codigo = CodigoExecutavel::novo(&bytes).ok()?;
    let frios = instrs
        .iter()
        .filter_map(|i| match i.op {
            Op::LeProp | Op::GravaProp | Op::DefProp => Some(i.c),
            Op::LeGlobal => Some(i.b),
            Op::LePropAcc => Some(i.b),
            _ => None,
        })
        .filter(|&ci| m.protos[proto].caches.get(ci as usize).is_some_and(|c| c.forma == u32::MAX))
        .collect();
    Some(CodigoJit { codigo, regs, proto, entradas, frios })
}

/// Caminho lento de `Soma r, c` (eax = registrador, r14d = acc): anota o perfil, se o cache
/// ainda não está estável, e deixa eax como estava.
fn anotar_soma(a: &mut Montador, m: &Motor, proto: usize, ins: &Instr) {
    let Some(t) = m.protos[proto].tipos.get(ins.b as usize) else { return };
    if *t >= crate::funcoes::tipo::ESTAVEL {
        return;
    }
    // O byte do perfil, lido direto: a lista de tipos da função não muda de tamanho nem de lugar
    // (as funções nunca saem de `Motor::protos`).
    let pula = a.rotulo();
    a.mov_imm(Tam::B64, Reg::R9, t as *const u8 as i64);
    a.movzx8(Tam::B32, Reg::R8, Mem::base(Reg::R9, 0));
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, i32::from(crate::funcoes::tipo::ESTAVEL));
    a.jcc(Cond::Ae, pula);
    a.mov(Tam::B64, Reg::Rdi, Reg::Rbx);
    a.mov_imm(Tam::B32, Reg::Rsi, i64::from(ins.b));
    a.mov(Tam::B32, Reg::Rdx, Reg::Rax);
    a.mov(Tam::B32, Reg::Rcx, Reg::R14);
    a.mov_imm(Tam::B64, Reg::Rax, anotar as extern "C" fn(*mut Contexto, u32, u32, u32) as usize as i64);
    a.call(Reg::Rax);
    recarregar(a);
    a.carregar(Tam::B32, Reg::Rax, m_reg(ins.a));
    a.ligar(pula);
}

/// `LeProp`/`LePropAcc` pelo cache `ci` da função `proto` (o objeto em eax; o valor no
/// acumulador). Falso = o cache não dá um caminho rápido (nada foi emitido).
fn ler_prop(a: &mut Montador, m: &Motor, proto: usize, k: u32, ci: u32, lento: Rotulo) -> bool {
    let Some(como) = leitura(m, proto, k, ci) else { return false };
    // rcx = deslocamento do objeto, r8d = forma.
    forma_de(a, Reg::Rax, lento);
    match como {
        Leitura::Campo(forma, desl) => {
            a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, forma as i32);
            a.jcc(Cond::Ne, lento);
            a.carregar(Tam::B32, Reg::R14, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, desl));
        }
        Leitura::Proto(forma, forma_dono, desl) => {
            a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, forma as i32);
            a.jcc(Cond::Ne, lento);
            // O protótipo (da tabela de formas) e a forma dele.
            a.carregar(Tam::B64, Reg::R9, Mem::base(Reg::Rbx, C_FORMAS));
            a.mov_imm(Tam::B64, Reg::R10, i64::from(forma) * std::mem::size_of::<Forma>() as i64);
            a.carregar(Tam::B32, Reg::Rcx, Mem::indexado(Reg::R9, Reg::R10, Escala::E1, std::mem::offset_of!(Forma, proto) as i32));
            a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
            a.carregar(Tam::B32, Reg::R8, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0));
            a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, forma_dono as i32);
            a.jcc(Cond::Ne, lento);
            a.carregar(Tam::B32, Reg::R14, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, desl));
        }
        Leitura::LengthArray => {
            conferir_forma(a, lento, Some(Classe::Array));
            // `length` cabe em 31 bits? (Os maiores vão em caixa.)
            a.carregar(Tam::B32, Reg::Rax, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 12));
            a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rax, 0x3FFF_FFFF);
            a.jcc(Cond::A, lento);
            a.alu(Alu::Add, Tam::B32, Reg::Rax, Reg::Rax);
            a.mov(Tam::B32, Reg::R14, Reg::Rax);
        }
    }
    true
}

/// O laço que guarda um registrador em ebp, como int32 sem caixa: (índice do começo, índice
/// do `SaltaAtras`, registrador). É o laço mais de dentro que roda inteiro no código, sem
/// chamadas, com `r = (r + x) | 0` e nenhuma outra instrução que leia ou grave `r`. Depois de
/// cada `Guarda r` o acumulador está morto (ninguém o lê antes de outra instrução o trocar):
/// assim o valor de `r` não precisa virar valor (com caixa) a cada volta. Só instruções cujo
/// caminho rápido não chama nativas: as nativas ficam nos caminhos lentos, que gravam `r`
/// antes.
fn laco_promovido(
    instrs: &[Instr],
    sabidas: &[bool],
    entradas: &[u32],
    dono_em: &dyn Fn(usize) -> Option<u32>,
    toca: &dyn Fn(&Instr, u32) -> bool,
    alvo_de_salto: &dyn Fn(usize) -> bool,
) -> Option<(usize, usize, u32)> {
    let compara = |op: Op| matches!(op, Op::Menor | Op::Maior | Op::MenorIgual | Op::MaiorIgual | Op::Igual | Op::Diferente | Op::Identico | Op::NaoIdentico);
    let salto = |op: Op| matches!(op, Op::Salta | Op::SaltaAtras | Op::SaltaSeFalso | Op::SaltaSeVerdadeiro);
    'lacos: for (j, sa) in instrs.iter().enumerate() {
        if sa.op != Op::SaltaAtras || !entradas.contains(&sa.a) {
            continue;
        }
        let Some(ini) = instrs.iter().position(|x| x.ini == sa.a as usize) else { continue };
        if ini >= j || !sabidas[ini..=j].iter().all(|&s| s) {
            continue;
        }
        let (h, b) = (instrs[ini].ini, sa.ini);
        let mut r = None;
        let mut k = ini;
        while k <= j {
            let x = &instrs[k];
            match x.op {
                op if compara(op) => {
                    let ok = k + 1 < j && matches!(instrs[k + 1].op, Op::SaltaSeFalso | Op::SaltaSeVerdadeiro) && !alvo_de_salto(instrs[k + 1].ini);
                    if !ok {
                        continue 'lacos;
                    }
                    k += 2;
                    continue;
                }
                Op::Soma => {
                    let Some(d) = dono_em(k) else { continue 'lacos };
                    if r.is_some_and(|r| r != d) || k + 3 > j {
                        continue 'lacos;
                    }
                    // O acumulador morto depois do `Guarda`.
                    let depois = &instrs[k + 3];
                    let morto = k + 3 == j || (matches!(depois.op, Op::IncrementaReg | Op::DecrementaReg) && depois.a != d && k + 4 == j);
                    if !morto {
                        continue 'lacos;
                    }
                    r = Some(d);
                    k += 3;
                    continue;
                }
                Op::SaltaAtras if k != j => continue 'lacos,
                Op::CarregaInt
                | Op::CarregaUndef
                | Op::CarregaVerdadeiro
                | Op::CarregaFalso
                | Op::Carrega
                | Op::Guarda
                | Op::Move
                | Op::IncrementaReg
                | Op::DecrementaReg
                | Op::Incrementa
                | Op::Decrementa
                | Op::ParaNumerico
                | Op::ParaInt32
                | Op::Sub
                | Op::Mul
                | Op::E
                | Op::Ou
                | Op::OuExclusivo
                | Op::Resto
                | Op::Salta
                | Op::SaltaAtras => {}
                _ => continue 'lacos,
            }
            k += 1;
        }
        let Some(r) = r else { continue };
        // O começo do laço troca o acumulador sem lê-lo (a volta chega com ele morto).
        if !matches!(instrs[ini].op, Op::CarregaInt | Op::Carrega | Op::CarregaUndef | Op::CarregaVerdadeiro | Op::CarregaFalso) {
            continue;
        }
        // Só as somas promovidas tocam `r`.
        let mut k = ini;
        while k <= j {
            if dono_em(k) == Some(r) {
                k += 3;
                continue;
            }
            if toca(&instrs[k], r) {
                continue 'lacos;
            }
            k += 1;
        }
        // De fora, só se entra pelo começo.
        if instrs.iter().enumerate().any(|(k, x)| (k < ini || k > j) && salto(x.op) && (x.a as usize) > h && (x.a as usize) <= b) {
            continue;
        }
        if entradas.iter().any(|&e| (e as usize) > h && (e as usize) <= b) {
            continue;
        }
        return Some((ini, j, r));
    }
    None
}

/// ebp = o registrador `r` como int32 (inteiro marcado ou caixa com int32 que não é -0);
/// outro valor sai por `falha`.
fn carregar_int32(a: &mut Montador, r: u32, falha: Rotulo) {
    let caixa = a.rotulo();
    let pronto = a.rotulo();
    let nao_zero = a.rotulo();
    a.carregar(Tam::B32, Reg::Rax, m_reg(r));
    a.test_imm(Tam::B32, Reg::Rax, 1);
    a.jcc(Cond::Ne, caixa);
    a.desloc(Desloc::Sar, Tam::B32, Reg::Rax, 1);
    a.mov(Tam::B32, Reg::Rbp, Reg::Rax);
    a.jmp(pronto);
    a.ligar(caixa);
    a.mov(Tam::B32, Reg::Rcx, Reg::Rax);
    a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
    a.carregar(Tam::B32, Reg::R8, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0));
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, crate::formas::NUMERO.0 as i32);
    a.jcc(Cond::Ne, falha);
    a.movsd_carregar(Xmm::X0, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 8));
    a.cvttsd2si(Reg::Rsi, Xmm::X0);
    a.cvtsi2sd(Xmm::X1, Reg::Rsi);
    a.ucomisd(Xmm::X0, Xmm::X1);
    a.jcc(Cond::Ne, falha);
    a.jcc(Cond::P, falha);
    a.movsxd(Reg::Rcx, Reg::Rsi);
    a.alu(Alu::Cmp, Tam::B64, Reg::Rcx, Reg::Rsi);
    a.jcc(Cond::Ne, falha);
    a.test(Tam::B32, Reg::Rsi, Reg::Rsi);
    a.jcc(Cond::Ne, nao_zero);
    a.movq_de_xmm(Reg::Rdx, Xmm::X0);
    a.test(Tam::B64, Reg::Rdx, Reg::Rdx);
    a.jcc(Cond::S, falha);
    a.ligar(nao_zero);
    a.mov(Tam::B32, Reg::Rbp, Reg::Rsi);
    a.ligar(pronto);
}

/// Grava o int32 de ebp no registrador `r` como valor (marcado, ou numa caixa nova), sem mudar
/// o acumulador.
fn materializar(a: &mut Montador, r: u32) {
    let grande = a.rotulo();
    let pronto = a.rotulo();
    a.mov(Tam::B32, Reg::Rax, Reg::Rbp);
    a.alu(Alu::Add, Tam::B32, Reg::Rax, Reg::Rax);
    a.jcc(Cond::O, grande);
    a.guardar(Tam::B32, m_reg(r), Reg::Rax);
    a.jmp(pronto);
    a.ligar(grande);
    // A caixa sai em r14 (e pode chamar a nativa): o acumulador fica guardado na pilha.
    a.alu_imm(Alu::Sub, Tam::B64, Reg::Rsp, 16);
    a.guardar(Tam::B64, Mem::base(Reg::Rsp, 0), Reg::R14);
    a.movsxd(Reg::Rsi, Reg::Rbp);
    a.cvtsi2sd(Xmm::X0, Reg::Rsi);
    caixa_de_xmm0(a);
    a.guardar(Tam::B32, m_reg(r), Reg::R14);
    a.carregar(Tam::B64, Reg::R14, Mem::base(Reg::Rsp, 0));
    a.alu_imm(Alu::Add, Tam::B64, Reg::Rsp, 16);
    a.ligar(pronto);
}

/// `SaltaSeFalso`/`SaltaSeVerdadeiro` com qualquer valor no acumulador: salta para `alvo`, ou
/// segue (para `segue`, ou para depois, se o booleano saiu do Rust).
fn salta_se(a: &mut Montador, ins: &Instr, alvo: Rotulo, segue: Rotulo, aborto: Rotulo) {
    let (salta, nao) = if ins.op == Op::SaltaSeFalso { (FALSE, TRUE) } else { (TRUE, FALSE) };
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R14, salta);
    a.jcc(Cond::E, alvo);
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R14, nao);
    a.jcc(Cond::E, segue);
    // Outro valor: o booleano dele pelo Rust, sem mudar o acumulador.
    chamar_instrucao(a, ins, aborto);
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::Rax, salta);
    a.jcc(Cond::E, alvo);
}

/// Gravar o acumulador num campo do objeto em `d` (deslocamento) dispensa a barreira de
/// escrita? Sim se o valor não é referência ou o objeto está no berçário; senão vai a `lento`
/// (a nativa grava com a barreira).
fn barreira(a: &mut Montador, m: &Motor, d: Reg, lento: Rotulo) {
    let ok = a.rotulo();
    a.test_imm(Tam::B32, Reg::R14, 1);
    a.jcc(Cond::E, ok);
    a.alu_imm(Alu::Cmp, Tam::B32, d, m.g.bercario_inicio as i32);
    a.jcc(Cond::B, lento);
    a.alu_imm(Alu::Cmp, Tam::B32, d, m.g.bercario_fim as i32);
    a.jcc(Cond::Ae, lento);
    a.ligar(ok);
}

/// `reg` = endereço do quadro deste código (`Motor::quadros[ctx.quadro]`); rdx = o motor.
fn quadro_de(a: &mut Montador, reg: Reg) {
    a.carregar(Tam::B32, reg, Mem::base(Reg::Rbx, C_QUADRO));
    a.imul_imm(Tam::B64, reg, reg, TAM_QUADRO);
    a.alu_mem(Alu::Add, Tam::B64, reg, Mem::base(Reg::Rdx, OFF_QUADROS + P_PTR));
}

/// r12 = registradores, r13 = gaiola (de novo do contexto: uma nativa pode tê-los mudado).
fn recarregar(a: &mut Montador) {
    a.carregar(Tam::B64, Reg::R12, Mem::base(Reg::Rbx, C_REGS));
    a.carregar(Tam::B64, Reg::R13, Mem::base(Reg::Rbx, C_GAIOLA));
}

/// rcx = deslocamento do objeto em `src` e r8d = a forma dele; não referência sai por `falha`.
fn forma_de(a: &mut Montador, src: Reg, falha: Rotulo) {
    a.test_imm(Tam::B32, src, 1);
    a.jcc(Cond::E, falha);
    a.mov(Tam::B32, Reg::Rcx, src);
    a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
    a.carregar(Tam::B32, Reg::R8, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0));
}

/// A forma em r8d é de objeto do JS (e da `classe`, se dada); senão sai por `falha`. Usa r9 e
/// r11 (r9 = a tabela compacta de tipo e classe).
fn conferir_forma(a: &mut Montador, falha: Rotulo, classe: Option<Classe>) {
    a.alu_mem(Alu::Cmp, Tam::B32, Reg::R8, Mem::base(Reg::Rbx, C_N_FORMAS));
    a.jcc(Cond::Ae, falha);
    a.carregar(Tam::B64, Reg::R9, Mem::base(Reg::Rbx, C_CLASSES));
    a.movzx8(Tam::B32, Reg::R11, Mem::indexado(Reg::R9, Reg::R8, Escala::E2, 0));
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, Tipo::Objeto as i32);
    a.jcc(Cond::Ne, falha);
    if let Some(c) = classe {
        a.movzx8(Tam::B32, Reg::R11, Mem::indexado(Reg::R9, Reg::R8, Escala::E2, 1));
        a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, c as i32);
        a.jcc(Cond::Ne, falha);
    }
}

/// `dst` = o número em `src` como double (inteiro marcado ou número em caixa); outro valor sai
/// por `falha`. Usa rcx e r8.
fn numero_para_xmm(a: &mut Montador, src: Reg, dst: Xmm, falha: Rotulo) {
    let caixa = a.rotulo();
    let pronto = a.rotulo();
    a.test_imm(Tam::B32, src, 1);
    a.jcc(Cond::Ne, caixa);
    a.mov(Tam::B32, Reg::Rcx, src);
    a.desloc(Desloc::Sar, Tam::B32, Reg::Rcx, 1);
    a.movsxd(Reg::Rcx, Reg::Rcx);
    a.cvtsi2sd(dst, Reg::Rcx);
    a.jmp(pronto);
    a.ligar(caixa);
    a.mov(Tam::B32, Reg::Rcx, src);
    a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
    a.carregar(Tam::B32, Reg::R8, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 0));
    a.alu_imm(Alu::Cmp, Tam::B32, Reg::R8, crate::formas::NUMERO.0 as i32);
    a.jcc(Cond::Ne, falha);
    a.movsd_carregar(dst, Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 8));
    a.ligar(pronto);
}

/// acc = int32 em `esi`: marcado se cabe em 31 bits, senão em caixa. Com `dono` (o registrador
/// que vai receber o resultado): se ele tem a caixa própria do código, o número vai nela.
fn marcar_ou_caixa(a: &mut Montador, dono: Option<u32>) {
    let caixa = a.rotulo();
    let pronto = a.rotulo();
    a.mov(Tam::B32, Reg::Rax, Reg::Rsi);
    a.alu(Alu::Add, Tam::B32, Reg::Rax, Reg::Rax);
    a.jcc(Cond::O, caixa);
    a.mov(Tam::B32, Reg::R14, Reg::Rax);
    a.jmp(pronto);
    a.ligar(caixa);
    a.movsxd(Reg::Rsi, Reg::Rsi);
    a.cvtsi2sd(Xmm::X0, Reg::Rsi);
    if let Some(r) = dono {
        let nova = a.rotulo();
        a.carregar(Tam::B32, Reg::Rax, m_reg(r));
        a.alu_mem(Alu::Cmp, Tam::B32, Reg::Rax, Mem::base(Reg::Rbx, C_CAIXA));
        a.jcc(Cond::Ne, nova);
        a.test(Tam::B32, Reg::Rax, Reg::Rax);
        a.jcc(Cond::E, nova);
        // A mesma caixa, com o número novo (só este registrador a tem).
        a.mov(Tam::B32, Reg::Rcx, Reg::Rax);
        a.alu_imm(Alu::And, Tam::B32, Reg::Rcx, -2);
        a.movsd_guardar(Mem::indexado(Reg::R13, Reg::Rcx, Escala::E1, 8), Xmm::X0);
        a.mov(Tam::B32, Reg::R14, Reg::Rax);
        a.jmp(pronto);
        a.ligar(nova);
        caixa_de_xmm0(a);
        a.guardar(Tam::B32, Mem::base(Reg::Rbx, C_CAIXA), Reg::R14);
    } else {
        caixa_de_xmm0(a);
    }
    a.ligar(pronto);
}

/// acc = o double em xmm0 como valor (`Motor::numero`): inteiro marcado se é inteiro de 31 bits
/// (e não -0), senão em caixa.
fn numero_de_xmm0(a: &mut Montador) {
    let caixa = a.rotulo();
    let pronto = a.rotulo();
    a.cvttsd2si(Reg::Rcx, Xmm::X0);
    a.cvtsi2sd(Xmm::X1, Reg::Rcx);
    a.ucomisd(Xmm::X0, Xmm::X1);
    a.jcc(Cond::Ne, caixa);
    a.jcc(Cond::P, caixa);
    // Dentro de [-2^30, 2^30).
    a.lea(Reg::Rdx, Mem::base(Reg::Rcx, 1 << 30));
    a.mov_imm(Tam::B64, Reg::R8, 1 << 31);
    a.alu(Alu::Cmp, Tam::B64, Reg::Rdx, Reg::R8);
    a.jcc(Cond::Ae, caixa);
    // Zero: -0 vai em caixa (o bit de sinal do double).
    let marca = a.rotulo();
    a.test(Tam::B32, Reg::Rcx, Reg::Rcx);
    a.jcc(Cond::Ne, marca);
    a.movq_de_xmm(Reg::Rdx, Xmm::X0);
    a.test(Tam::B64, Reg::Rdx, Reg::Rdx);
    a.jcc(Cond::S, caixa);
    a.ligar(marca);
    a.alu(Alu::Add, Tam::B32, Reg::Rcx, Reg::Rcx);
    a.mov(Tam::B32, Reg::R14, Reg::Rcx);
    a.jmp(pronto);
    a.ligar(caixa);
    caixa_de_xmm0(a);
    a.ligar(pronto);
}

/// acc = caixa nova com o double de xmm0, no berçário (como `Motor::caixa_numero`); berçário
/// cheio vai à nativa.
fn caixa_de_xmm0(a: &mut Montador) {
    let nativa = a.rotulo();
    let pronto = a.rotulo();
    a.carregar(Tam::B64, Reg::Rdx, Mem::base(Reg::Rbx, C_TOPO));
    a.carregar(Tam::B32, Reg::Rax, Mem::base(Reg::Rdx, 0));
    a.lea(Reg::Rcx, Mem::base(Reg::Rax, 16));
    a.carregar(Tam::B64, Reg::R8, Mem::base(Reg::Rbx, C_TETO));
    a.alu_mem(Alu::Cmp, Tam::B32, Reg::Rcx, Mem::base(Reg::R8, 0));
    a.jcc(Cond::A, nativa);
    a.guardar(Tam::B32, Mem::base(Reg::Rdx, 0), Reg::Rcx);
    a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 0), crate::formas::NUMERO.0 as i32);
    a.guardar_imm(Tam::B32, Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 4), 0);
    a.movsd_guardar(Mem::indexado(Reg::R13, Reg::Rax, Escala::E1, 8), Xmm::X0);
    a.alu_imm(Alu::Or, Tam::B32, Reg::Rax, 1);
    a.mov(Tam::B32, Reg::R14, Reg::Rax);
    a.jmp(pronto);
    a.ligar(nativa);
    chamar_nativa(a, caixa_f64 as extern "C" fn(*mut Contexto, f64) -> u32 as usize);
    a.mov(Tam::B32, Reg::R14, Reg::Rax);
    a.ligar(pronto);
}

/// Chama `f(ctx, …)` (os outros argumentos já nos registradores) e recarrega os ponteiros.
fn chamar_nativa(a: &mut Montador, f: usize) {
    a.mov(Tam::B64, Reg::Rdi, Reg::Rbx);
    a.mov_imm(Tam::B64, Reg::Rax, f as i64);
    a.call(Reg::Rax);
    recarregar(a);
}

/// Chama `f`, que devolve o valor (em eax) ou [`ABORTA`] (sai por `aborto`).
fn resultado_de_nativa(a: &mut Montador, f: usize, aborto: Rotulo) {
    a.mov_imm(Tam::B64, Reg::Rax, f as i64);
    a.call(Reg::Rax);
    recarregar(a);
    a.mov(Tam::B64, Reg::Rcx, Reg::Rax);
    a.desloc(Desloc::Shr, Tam::B64, Reg::Rcx, 32);
    a.jcc(Cond::Ne, aborto);
}

/// O caminho geral de `ins` pela nativa [`instrucao`]: o resultado em eax (o acumulador não muda
/// aqui).
fn chamar_instrucao(a: &mut Montador, ins: &Instr, aborto: Rotulo) {
    // rdi = ctx, esi = op, edx = a, ecx = b, r8d = c, r9d = ini; o acumulador vai no contexto.
    a.guardar(Tam::B32, Mem::base(Reg::Rbx, C_ACC), Reg::R14);
    a.mov(Tam::B64, Reg::Rdi, Reg::Rbx);
    a.mov_imm(Tam::B32, Reg::Rsi, i64::from(ins.op as u8));
    a.mov_imm(Tam::B32, Reg::Rdx, i64::from(ins.a));
    a.mov_imm(Tam::B32, Reg::Rcx, i64::from(ins.b));
    a.mov_imm(Tam::B32, Reg::R8, i64::from(ins.c));
    a.mov_imm(Tam::B32, Reg::R9, ins.ini as i64);
    let f = instrucao as extern "C" fn(*mut Contexto, u32, u32, u32, u32, u32) -> u64;
    resultado_de_nativa(a, f as usize, aborto);
}

/// eax = registrador `r`, e ele e o acumulador são inteiros marcados (senão vai a `lento`).
/// Deixa ecx = os dois em `or`.
fn dois_inteiros(a: &mut Montador, r: u32, lento: Rotulo) {
    a.carregar(Tam::B32, Reg::Rax, m_reg(r));
    a.mov(Tam::B32, Reg::Rcx, Reg::Rax);
    a.alu(Alu::Or, Tam::B32, Reg::Rcx, Reg::R14);
    a.test_imm(Tam::B32, Reg::Rcx, 1);
    a.jcc(Cond::Ne, lento);
}

/// A forma em r8d é de objeto do JS de uma das `classes` (r11d = a classe); senão `falha`.
fn conferir_classe(a: &mut Montador, falha: Rotulo, classes: &[Classe]) {
    conferir_forma(a, falha, None);
    a.movzx8(Tam::B32, Reg::R11, Mem::indexado(Reg::R9, Reg::R8, Escala::E2, 1));
    let ok = a.rotulo();
    for &c in classes {
        a.alu_imm(Alu::Cmp, Tam::B32, Reg::R11, c as i32);
        a.jcc(Cond::E, ok);
    }
    a.jmp(falha);
    a.ligar(ok);
}

#[cfg(test)]
mod testes {
    use crate::motor::Motor;

    fn nos_dois(fonte: &str) -> String {
        let mut saidas = Vec::new();
        for modo in [0, 2] {
            let mut m = Motor::novo();
            m.jit_modo = modo;
            let v = m.avaliar(fonte).unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
            saidas.push(m.para_texto_rust(v).unwrap());
        }
        assert_eq!(saidas[0], saidas[1], "N1 e N2 diferem");
        saidas.pop().unwrap()
    }

    /// O mesmo script no N1 e com tudo no N2 dá o mesmo resultado (guardas, laços aninhados que
    /// passam do lote, propriedade de forma trocada, buraco, caixa de número).
    #[test]
    fn n2_igual_ao_n1() {
        let r = nos_dois(
            "function P(x){this.x=x}
            const a=[new P(1),{x:2},new P(3)], h=[1,,3], r=[];
            let s=0; for (let k=0;k<400;k++) for (let i=0;i<a.length;i++) s=(s+a[i].x+k)|0; r.push(s);
            let u=0; for (let i=0;i<600;i++) u=u+(h[i%3]===undefined?0.5:h[i%3]); r.push(u);
            let w=1073741000; for (let i=0;i<3000;i++) w=w+i; r.push(w);
            let z=0; for (let i=0;i<3000;i++) z=(z+2147483000+i)|0; r.push(z);
            let o={get x(){return 7}}, b=[new P(1)], q=0; for (let i=0;i<600;i++){ q=q+b[0].x; if(i==300) b[0]=o; } r.push(q);
            r.join()",
        );
        assert_eq!(r, "241800,900,1078239500,2554500,2394");
    }

    /// O laço promovido (`s = (s + x) | 0` com s em ebp): começo em caixa, -0 e texto,
    /// `break`, o caminho lento (valueOf, que também lança) e o lote de voltas.
    #[test]
    fn n2_laco_promovido() {
        let r = nos_dois(
            "function f(s, n, x){ for (let i = 0; i < n; i++) { s = (s + i) | 0; } return s; }
            function g(s, n){ for (let i = 0; i < n; i++) { if (i == 7) break; s = (s + x) | 0; } return s; }
            var x = 3;
            const o = { valueOf(){ return 5; } };
            function h(n){ let s = 0; for (let i = 0; i < n; i++) { s = (s + (i == 50 ? o : i)) | 0; } return s; }
            function k(n){ let s = 0; try { for (let i = 0; i < n; i++) { s = (s + (i == 9 ? {valueOf(){ throw 1; }} : i)) | 0; } } catch (e) { return 'pego ' + s; } }
            [f(0, 100000), f(2147483000, 10000), Object.is(f(-0, 0), -0), f('a', 0), f(1.5, 3), g(10, 100), h(10000), k(100), f(0, 3e5)].join()",
        );
        assert_eq!(r, "704982704,-2097489296,true,a,4,31,49994955,pego 36,2050177040");
    }

    /// Chamadas de código para código: recursão, closure, exceção atravessando quadros do N2,
    /// função chamada que sai para o N1 no meio, nativa, construtor e recursão funda.
    #[test]
    fn n2_chamadas() {
        let r = nos_dois(
            "function fib(n){ return n < 2 ? n : fib(n-1) + fib(n-2); }
            function cont(){ let c = 0; return { inc: () => ++c, val: () => c }; }
            const k = cont(); for (let i=0;i<500;i++) k.inc();
            function lanca(n){ if (n == 0) throw new Error('fundo'); return lanca(n-1) + 1; }
            let pego = ''; try { lanca(50); } catch (e) { pego = e.message; }
            function mista(n){ return n < 300 ? n + 1 : ('t' + n).length; }
            let m = 0; for (let i=0;i<600;i++) m = m + mista(i);
            let nat = 0; for (let i=0;i<300;i++) nat = nat + Math.max(i, 7);
            function P(x){ this.x = x; } let ps = 0; for (let i=0;i<300;i++) ps = ps + new P(i).x;
            function fundo(n){ return n == 0 ? 0 : fundo(n-1) + 1; }
            [fib(20), k.val(), pego, m, nat, ps, fundo(3000)].join()",
        );
        assert_eq!(r, "6765,500,fundo,46350,44878,44850,3000");
    }
}
