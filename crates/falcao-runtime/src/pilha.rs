//! Pilha de valores com formato fixo na memória (`ptr`, `len`, `cap`, nesta ordem, `repr(C)`):
//! o código do N2 empurra e desempilha quadros direto, sem chamar o Rust (o `Vec` não garante o
//! formato dos campos). Fora isso é um `Vec` de tipo `Copy`: as operações que o motor usa, e
//! fatia (`Deref`) para o resto.

use std::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use std::ops::{Deref, DerefMut, RangeBounds};

#[repr(C)]
pub struct Pilha<T: Copy> {
    ptr: *mut T,
    len: usize,
    cap: usize,
}

/// Deslocamentos dos campos, para o código gerado.
pub const P_PTR: i32 = 0;
pub const P_LEN: i32 = 8;
pub const P_CAP: i32 = 16;

impl<T: Copy> Pilha<T> {
    pub fn com_capacidade(cap: usize) -> Self {
        let mut p = Pilha { ptr: std::ptr::NonNull::dangling().as_ptr(), len: 0, cap: 0 };
        p.reserve(cap);
        p
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Garante lugar para mais `n` itens.
    #[inline]
    pub fn reserve(&mut self, n: usize) {
        let precisa = self.len.checked_add(n).expect("pilha grande demais");
        if precisa > self.cap {
            self.crescer(precisa);
        }
    }

    #[cold]
    fn crescer(&mut self, precisa: usize) {
        assert!(std::mem::size_of::<T>() != 0);
        let nova = precisa.max(self.cap * 2).max(16);
        let layout = Layout::array::<T>(nova).expect("pilha grande demais");
        // SEGURANÇA: layout não vazio; o ponteiro antigo (se há) veio de `alloc`/`realloc` com
        // o layout de `self.cap`.
        let ptr = unsafe {
            if self.cap == 0 {
                alloc(layout)
            } else {
                realloc(self.ptr as *mut u8, Layout::array::<T>(self.cap).unwrap(), layout.size())
            }
        };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        self.ptr = ptr as *mut T;
        self.cap = nova;
    }

    /// O lugar livre no fim, com `n` itens garantidos: quem escreve neles chama
    /// [`Pilha::crescer_len`] depois.
    #[inline(always)]
    pub fn reservar_fim(&mut self, n: usize) -> *mut T {
        if self.cap - self.len < n {
            self.crescer(self.len.checked_add(n).expect("pilha grande demais"));
        }
        // SEGURANÇA: len ≤ cap (o ponteiro fica dentro do bloco ou um depois do fim).
        unsafe { self.ptr.add(self.len) }
    }

    /// Conta como ocupados os `n` itens escritos a partir do fim.
    ///
    /// # Safety
    /// Os `n` itens a partir de [`Pilha::reservar_fim`] foram escritos (e couberam nele).
    #[inline(always)]
    pub unsafe fn crescer_len(&mut self, n: usize) {
        debug_assert!(self.len + n <= self.cap);
        self.len += n;
    }

    #[inline(always)]
    pub fn push(&mut self, v: T) {
        if self.len == self.cap {
            self.crescer(self.len + 1);
        }
        // SEGURANÇA: len < cap depois do crescer.
        unsafe { self.ptr.add(self.len).write(v) };
        self.len += 1;
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SEGURANÇA: o item `len` estava inicializado.
        Some(unsafe { self.ptr.add(self.len).read() })
    }

    #[inline(always)]
    pub fn truncate(&mut self, len: usize) {
        if len < self.len {
            self.len = len;
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// `resize` de `Vec` (só cresce com `v` ou corta).
    #[inline]
    pub fn resize(&mut self, len: usize, v: T) {
        if len <= self.len {
            self.len = len;
            return;
        }
        self.reserve(len - self.len);
        while self.len < len {
            // SEGURANÇA: len < cap (reservado acima).
            unsafe { self.ptr.add(self.len).write(v) };
            self.len += 1;
        }
    }

    #[inline]
    pub fn extend_from_slice(&mut self, s: &[T]) {
        self.reserve(s.len());
        // SEGURANÇA: lugar reservado; `s` não pode apontar para dentro desta pilha além de `len`
        // (é uma fatia emprestada, e o empréstimo mutável de `self` exclui isso).
        unsafe { std::ptr::copy_nonoverlapping(s.as_ptr(), self.ptr.add(self.len), s.len()) };
        self.len += s.len();
    }

    /// Copia `faixa` (da própria pilha) para o fim.
    pub fn extend_from_within<R: RangeBounds<usize>>(&mut self, faixa: R) {
        use std::ops::Bound;
        let ini = match faixa.start_bound() {
            Bound::Included(&i) => i,
            Bound::Excluded(&i) => i + 1,
            Bound::Unbounded => 0,
        };
        let fim = match faixa.end_bound() {
            Bound::Included(&i) => i + 1,
            Bound::Excluded(&i) => i,
            Bound::Unbounded => self.len,
        };
        assert!(ini <= fim && fim <= self.len, "faixa fora da pilha");
        let faixa = ini..fim;
        let n = faixa.len();
        self.reserve(n);
        // SEGURANÇA: a origem está em [0, len) e o destino em [len, len+n) (reservado): não se
        // sobrepõem.
        unsafe { std::ptr::copy_nonoverlapping(self.ptr.add(faixa.start), self.ptr.add(self.len), n) };
        self.len += n;
    }

    pub fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        let it = it.into_iter();
        self.reserve(it.size_hint().0);
        for v in it {
            self.push(v);
        }
    }
}

impl<T: Copy> Default for Pilha<T> {
    fn default() -> Self {
        Pilha { ptr: std::ptr::NonNull::dangling().as_ptr(), len: 0, cap: 0 }
    }
}

impl<T: Copy> Drop for Pilha<T> {
    fn drop(&mut self) {
        if self.cap != 0 {
            // SEGURANÇA: alocado com este layout em `crescer`.
            unsafe { dealloc(self.ptr as *mut u8, Layout::array::<T>(self.cap).unwrap()) };
        }
    }
}

impl<T: Copy> Deref for Pilha<T> {
    type Target = [T];
    #[inline(always)]
    fn deref(&self) -> &[T] {
        // SEGURANÇA: [0, len) inicializado; ptr alinhado e não nulo (dangling com len 0).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T: Copy> DerefMut for Pilha<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [T] {
        // SEGURANÇA: idem.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

#[cfg(test)]
mod testes {
    use super::Pilha;

    #[test]
    fn como_vec() {
        let mut p: Pilha<u32> = Pilha::default();
        let mut v: Vec<u32> = Vec::new();
        for i in 0..1000u32 {
            p.push(i);
            v.push(i);
            if i % 7 == 0 {
                let l = v.len();
                p.extend_from_within(l / 2..l);
                v.extend_from_within(l / 2..l);
            }
            if i % 13 == 0 {
                p.truncate(p.len() / 2);
                v.truncate(v.len() / 2);
                assert_eq!(p.pop(), v.pop());
            }
            if i % 29 == 0 {
                p.resize(p.len() + 5, 9);
                v.resize(v.len() + 5, 9);
                p.extend_from_slice(&[1, 2, 3]);
                v.extend_from_slice(&[1, 2, 3]);
                p.extend(std::iter::repeat_n(4, 3));
                v.extend(std::iter::repeat_n(4, 3));
            }
            assert_eq!(&p[..], &v[..]);
        }
        p[3] = 77;
        v[3] = 77;
        assert_eq!(&p[..], &v[..]);
    }
}
