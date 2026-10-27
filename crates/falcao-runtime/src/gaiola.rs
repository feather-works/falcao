//! A gaiola: toda a memória dos objetos de um contexto JS, num bloco só, endereçada por
//! deslocamentos de 32 bits (`ARQUITETURA.md`, seção 7). O bloco pode crescer e mudar de
//! endereço; os valores não mudam, porque são deslocamentos. Por isso **nenhum ponteiro cru
//! para dentro da gaiola sobrevive a uma alocação**: todo acesso passa por aqui, por
//! deslocamento. Este é o único módulo do runtime com `unsafe`.
//!
//! Regiões, em ordem:
//!
//! ```text
//!  [0, PERMANENTE)          objetos fixos (undefined, null, ..., textos de 1 caractere)
//!  [PERMANENTE, +BERÇÁRIO)  objetos novos, alocação por incremento
//!  [velho_inicio, ...)      sobreviventes e objetos grandes; cresce no fim
//! ```
//!
//! Cabeçalho de todo objeto: a palavra 0 é o número da forma ([`crate::formas::FormaId`]),
//! que diz o tipo e o layout para o coletor.

use crate::valor::Valor;

/// Bytes reservados para os objetos fixos.
pub const PERMANENTE: u32 = 64 * 1024;
/// Tamanho do berçário (objetos novos).
pub const BERCARIO: u32 = 4 * 1024 * 1024;
/// Objeto maior que isto nasce direto na geração velha.
pub const GRANDE: u32 = 64 * 1024;

pub struct Gaiola {
    mem: Vec<u64>,
    /// Próximo byte livre da região permanente (só durante a criação do contexto).
    topo_permanente: u32,
    pub bercario_inicio: u32,
    pub bercario_fim: u32,
    topo_bercario: u32,
    pub velho_inicio: u32,
    topo_velho: u32,
    /// Campos de objetos velhos que apontam para o berçário (barreira de escrita).
    pub lembrados: Vec<u32>,
    /// O berçário encheu: coletar no próximo ponto seguro.
    pub coleta_pendente: bool,
    /// Bytes alocados na geração velha desde a última coleta completa.
    pub velho_desde_coleta: u64,
}

#[inline(always)]
const fn arredondar(bytes: u32) -> u32 {
    (bytes + 7) & !7
}

impl Default for Gaiola {
    fn default() -> Self {
        Self::nova()
    }
}

impl Gaiola {
    pub fn nova() -> Self {
        let velho_inicio = PERMANENTE + BERCARIO;
        let inicial = velho_inicio + 4 * 1024 * 1024;
        Self {
            mem: vec![0u64; (inicial / 8) as usize],
            topo_permanente: 8,
            bercario_inicio: PERMANENTE,
            bercario_fim: PERMANENTE + BERCARIO,
            topo_bercario: PERMANENTE,
            velho_inicio,
            topo_velho: velho_inicio,
            lembrados: Vec::new(),
            coleta_pendente: false,
            velho_desde_coleta: 0,
        }
    }

    /// Bytes em uso (berçário + velha), para o F12 e para decidir coletas.
    pub fn em_uso(&self) -> u64 {
        u64::from(self.topo_bercario - self.bercario_inicio) + u64::from(self.topo_velho - self.velho_inicio)
    }

    pub fn tamanho_reservado(&self) -> usize {
        self.mem.len() * 8
    }

    // ---------------------------------------------------------------- alocação

    /// Aloca na região permanente (só na criação do contexto).
    pub fn alocar_permanente(&mut self, bytes: u32) -> u32 {
        let d = self.topo_permanente;
        let fim = d + arredondar(bytes);
        assert!(fim <= PERMANENTE, "região permanente cheia");
        self.topo_permanente = fim;
        d
    }

    /// Aloca `bytes` (zerados) e devolve o deslocamento. Nunca coleta aqui: quando o berçário
    /// enche, aloca na velha e marca [`Gaiola::coleta_pendente`].
    #[inline]
    pub fn alocar(&mut self, bytes: u32) -> u32 {
        let b = arredondar(bytes);
        let d = self.topo_bercario;
        if b < GRANDE && d + b <= self.bercario_fim {
            self.topo_bercario = d + b;
            return d;
        }
        if b < GRANDE {
            self.coleta_pendente = true;
        }
        self.alocar_velho(b)
    }

    pub fn alocar_velho(&mut self, bytes: u32) -> u32 {
        let b = arredondar(bytes);
        let d = self.topo_velho;
        let fim = d.checked_add(b).expect("gaiola cheia (4 GB)");
        if fim as usize > self.mem.len() * 8 {
            let novo = (fim as usize).max(self.mem.len() * 8 * 2).min(u32::MAX as usize & !7);
            assert!(novo >= fim as usize, "gaiola cheia (4 GB)");
            self.mem.resize(novo / 8, 0);
        }
        self.topo_velho = fim;
        self.velho_desde_coleta += u64::from(b);
        d
    }

    /// Zera o berçário inteiro (depois de copiar os sobreviventes).
    pub fn esvaziar_bercario(&mut self) {
        let (a, b) = ((self.bercario_inicio / 8) as usize, (self.topo_bercario / 8) as usize);
        self.mem[a..b].fill(0);
        self.topo_bercario = self.bercario_inicio;
        self.coleta_pendente = false;
    }

    pub fn topo_bercario(&self) -> u32 {
        self.topo_bercario
    }

    pub fn topo_velho(&self) -> u32 {
        self.topo_velho
    }

    /// Recua o topo da velha (depois de compactar).
    pub fn definir_topo_velho(&mut self, topo: u32) {
        let (a, b) = ((topo / 8) as usize, (self.topo_velho / 8) as usize);
        if a < b {
            self.mem[a..b].fill(0);
        }
        self.topo_velho = topo;
    }

    #[inline(always)]
    pub fn no_bercario(&self, d: u32) -> bool {
        d >= self.bercario_inicio && d < self.bercario_fim
    }

    // ---------------------------------------------------------------- acesso

    #[inline(always)]
    fn ptr(&self) -> *const u8 {
        self.mem.as_ptr() as *const u8
    }

    #[inline(always)]
    fn ptr_mut(&mut self) -> *mut u8 {
        self.mem.as_mut_ptr() as *mut u8
    }

    #[inline(always)]
    fn conferir(&self, d: u32, n: usize) {
        assert!(d as usize + n <= self.mem.len() * 8, "acesso fora da gaiola: {d}+{n}");
    }

    #[inline(always)]
    pub fn u32_em(&self, d: u32) -> u32 {
        self.conferir(d, 4);
        // SAFETY: dentro do bloco (conferido acima); leitura sem alinhamento exigido.
        unsafe { (self.ptr().add(d as usize) as *const u32).read_unaligned() }
    }

    #[inline(always)]
    pub fn por_u32(&mut self, d: u32, v: u32) {
        self.conferir(d, 4);
        // SAFETY: dentro do bloco (conferido acima).
        unsafe { (self.ptr_mut().add(d as usize) as *mut u32).write_unaligned(v) }
    }

    #[inline(always)]
    pub fn valor_em(&self, d: u32) -> Valor {
        Valor(self.u32_em(d))
    }

    /// Escreve um valor no campo `d` do objeto `obj`, com a barreira de escrita.
    #[inline(always)]
    pub fn por_valor(&mut self, obj: u32, d: u32, v: Valor) {
        self.por_u32(d, v.0);
        if v.e_ref() && !self.no_bercario(obj) && self.no_bercario(v.deslocamento()) && obj >= self.velho_inicio {
            self.lembrados.push(d);
        }
    }

    #[inline(always)]
    pub fn f64_em(&self, d: u32) -> f64 {
        self.conferir(d, 8);
        // SAFETY: dentro do bloco.
        unsafe { (self.ptr().add(d as usize) as *const f64).read_unaligned() }
    }

    #[inline(always)]
    pub fn por_f64(&mut self, d: u32, v: f64) {
        self.conferir(d, 8);
        // SAFETY: dentro do bloco.
        unsafe { (self.ptr_mut().add(d as usize) as *mut f64).write_unaligned(v) }
    }

    pub fn bytes(&self, d: u32, n: u32) -> &[u8] {
        self.conferir(d, n as usize);
        // SAFETY: dentro do bloco; a fatia empresta `self`, então nenhuma alocação a invalida.
        unsafe { std::slice::from_raw_parts(self.ptr().add(d as usize), n as usize) }
    }

    pub fn bytes_mut(&mut self, d: u32, n: u32) -> &mut [u8] {
        self.conferir(d, n as usize);
        // SAFETY: dentro do bloco; empréstimo exclusivo de `self`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr_mut().add(d as usize), n as usize) }
    }

    /// `n` unidades UTF-16 a partir de `d` (que tem de estar alinhado em 2).
    pub fn u16s(&self, d: u32, n: u32) -> &[u16] {
        debug_assert!(d % 2 == 0);
        self.conferir(d, n as usize * 2);
        // SAFETY: dentro do bloco, alinhado em 2 (cabeçalhos são múltiplos de 4).
        unsafe { std::slice::from_raw_parts(self.ptr().add(d as usize) as *const u16, n as usize) }
    }

    pub fn u16s_mut(&mut self, d: u32, n: u32) -> &mut [u16] {
        debug_assert!(d % 2 == 0);
        self.conferir(d, n as usize * 2);
        // SAFETY: dentro do bloco, alinhado em 2.
        unsafe { std::slice::from_raw_parts_mut(self.ptr_mut().add(d as usize) as *mut u16, n as usize) }
    }

    /// Copia `n` bytes de `de` para `para` (podem se sobrepor).
    pub fn copiar(&mut self, de: u32, para: u32, n: u32) {
        self.conferir(de, n as usize);
        self.conferir(para, n as usize);
        // SAFETY: as duas faixas estão dentro do bloco; `copy` aceita sobreposição.
        unsafe { std::ptr::copy(self.ptr().add(de as usize), self.ptr_mut().add(para as usize), n as usize) }
    }

    /// Forma (palavra 0) do objeto apontado por `v`.
    #[inline(always)]
    pub fn forma_de(&self, v: Valor) -> u32 {
        self.u32_em(v.deslocamento())
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn aloca_cresce_e_le() {
        let mut g = Gaiola::nova();
        let a = g.alocar(12);
        let b = g.alocar(8);
        assert_eq!(a % 8, 0);
        assert_eq!(b, a + 16);
        g.por_u32(a, 77);
        g.por_f64(b, 1.5);
        assert_eq!(g.u32_em(a), 77);
        assert_eq!(g.f64_em(b), 1.5);
        // Objeto grande vai para a velha; o bloco cresce sem mudar os deslocamentos.
        let c = g.alocar(10 * 1024 * 1024);
        assert!(c >= g.velho_inicio);
        g.por_u32(c + 10 * 1024 * 1024 - 4, 5);
        assert_eq!(g.u32_em(a), 77);
        // Barreira: objeto velho apontando para o berçário fica lembrado.
        g.por_valor(c, c + 8, Valor::de_deslocamento(a));
        assert_eq!(g.lembrados, vec![c + 8]);
    }
}
