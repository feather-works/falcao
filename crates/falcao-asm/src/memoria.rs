//! Memória executável W^X: mapeia leitura + escrita, copia o código e troca para leitura +
//! execução. Nunca as duas permissões juntas. Único lugar do crate com `unsafe`.

use crate::{Erro, MAX_CODIGO};

/// Código de máquina pronto para rodar (páginas `r-x`), desmapeado no `Drop`.
pub struct CodigoExecutavel {
    ptr: *mut u8,
    tamanho: usize,
}

impl CodigoExecutavel {
    /// Copia `codigo` para páginas novas e as deixa só leitura + execução.
    pub fn novo(codigo: &[u8]) -> Result<CodigoExecutavel, Erro> {
        if codigo.is_empty() || codigo.len() > MAX_CODIGO {
            return Err(Erro::TamanhoInvalido(codigo.len()));
        }
        let tamanho = codigo.len().next_multiple_of(sistema::PAGINA);
        let ptr = sistema::mapear_rw(tamanho)?;
        // SEGURANÇA: `ptr` aponta para `tamanho >= codigo.len()` bytes recém-mapeados com
        // escrita, só nossos, sem sobreposição com `codigo`.
        unsafe { std::ptr::copy_nonoverlapping(codigo.as_ptr(), ptr, codigo.len()) };
        let c = CodigoExecutavel { ptr, tamanho };
        // shortcut: x86 não precisa limpar o cache de instruções; AArch64 vai precisar aqui.
        sistema::proteger_rx(c.ptr, c.tamanho)?;
        Ok(c)
    }

    /// Endereço do início do código (para conferências e chamadas relativas).
    pub fn endereco(&self) -> usize {
        self.ptr as usize
    }

    /// Chama o código como `extern "C" fn(u64, u64) -> u64` (System V: `rdi`, `rsi` → `rax`).
    ///
    /// # Safety
    /// O código é arbitrário: quem chama garante que ele segue essa convenção de chamada,
    /// preserva os registradores salvos pelo chamado e só acessa memória válida.
    pub unsafe fn chamar2(&self, a: u64, b: u64) -> u64 {
        // SEGURANÇA: `ptr` está mapeado `r-x` enquanto `self` vive (o empréstimo garante), e o
        // contrato da função cobre o que o código faz.
        let f: extern "C" fn(u64, u64) -> u64 = unsafe { std::mem::transmute(self.ptr) };
        f(a, b)
    }

    /// Chama o código como `extern "C" fn(*mut T) -> u64`, com um contexto em `rdi`.
    ///
    /// # Safety
    /// Como em [`CodigoExecutavel::chamar2`]; além disso o código só pode acessar o contexto
    /// dentro de `T`.
    pub unsafe fn chamar_com<T>(&self, ctx: &mut T) -> u64 {
        // SEGURANÇA: idem `chamar2`; `ctx` é um ponteiro válido e exclusivo durante a chamada.
        let f: extern "C" fn(*mut T) -> u64 = unsafe { std::mem::transmute(self.ptr) };
        f(ctx)
    }
}

impl Drop for CodigoExecutavel {
    fn drop(&mut self) {
        sistema::desmapear(self.ptr, self.tamanho);
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod sistema {
    use crate::Erro;
    use std::ffi::c_void;

    pub const PAGINA: usize = 4096;
    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const PROT_EXEC: i32 = 4;
    const MAP_PRIVATE: i32 = 2;
    const MAP_ANONYMOUS: i32 = 0x20;

    extern "C" {
        fn mmap(addr: *mut c_void, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: i32) -> i32;
        fn munmap(addr: *mut c_void, len: usize) -> i32;
    }

    fn errno() -> Erro {
        Erro::Sistema(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }

    pub fn mapear_rw(tamanho: usize) -> Result<*mut u8, Erro> {
        // SEGURANÇA: mapeamento anônimo novo (sem endereço pedido, sem arquivo); o resultado é
        // conferido contra MAP_FAILED.
        let p = unsafe { mmap(std::ptr::null_mut(), tamanho, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0) };
        if p as isize == -1 {
            return Err(errno());
        }
        Ok(p.cast())
    }

    pub fn proteger_rx(ptr: *mut u8, tamanho: usize) -> Result<(), Erro> {
        // SEGURANÇA: `ptr`/`tamanho` vêm de `mapear_rw`; sai a escrita, entra a execução.
        if unsafe { mprotect(ptr.cast(), tamanho, PROT_READ | PROT_EXEC) } != 0 {
            return Err(errno());
        }
        Ok(())
    }

    pub fn desmapear(ptr: *mut u8, tamanho: usize) {
        // SEGURANÇA: chamado só pelo `Drop`, uma vez, com a região de `mapear_rw`.
        unsafe { munmap(ptr.cast(), tamanho) };
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod sistema {
    use crate::Erro;

    pub const PAGINA: usize = 4096;

    pub fn mapear_rw(_tamanho: usize) -> Result<*mut u8, Erro> {
        Err(Erro::SemSuporte)
    }

    pub fn proteger_rx(_ptr: *mut u8, _tamanho: usize) -> Result<(), Erro> {
        Err(Erro::SemSuporte)
    }

    pub fn desmapear(_ptr: *mut u8, _tamanho: usize) {}
}
