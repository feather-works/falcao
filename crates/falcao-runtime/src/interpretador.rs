//! Intérprete N1 (`ARQUITETURA.md`, seção 9): laço sobre o bytecode, com os quadros numa pilha
//! própria. Chamada JS→JS não usa a pilha do Rust: empurra um [`Quadro`] e continua no mesmo
//! laço. Nativas que chamam JS (`chamar`) abrem um laço novo, que termina quando o quadro de
//! entrada volta.
//!
//! Registradores do quadro moram em `Motor::pilha[base..fim]`; os parâmetros são os primeiros.
//! Se vieram mais argumentos que parâmetros, eles ficam logo antes de `base` (para `arguments`).

use crate::formas::{attr, Chave, Classe};
use crate::funcoes::{tipo, Args, CacheProp, ConstRt, Res, BIT_NATIVA, F_CASA, F_CODIGO, F_CONTEXTO, F_THIS, MEGA, SEM_POLI};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::operadores::{Arit, Bits};
use crate::textos::ChaveProp;
use crate::valor::Valor;
use falcao_bytecode::instrucoes::{ler_operando, Operando};
use falcao_bytecode::{Constante, Op};
use std::rc::Rc;

/// Endereço de uma variável local: o topo atual da pilha do Rust (ela cresce para baixo).
#[inline(always)]
fn endereco_pilha() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

/// Chamadas a partir das quais o N2 compila a função (os laços compilam pelo calor das voltas).
const LIMITE_JIT: u32 = 500;

/// Como o código do N2 terminou ([`Motor::rodar_jit`]).
enum SaidaJit {
    /// O intérprete segue em `pc` com o acumulador.
    Segue(usize, Valor),
    /// `Retorna` em `pc` com o valor no acumulador.
    Retorno(usize, Valor),
    /// Uma chamada aninhada saiu para o N1: o quadro do topo já está pronto (acumulador em
    /// `Motor::jit_acc`, erro em `Motor::jit_erro`).
    Aborto,
}

/// Sem gerador/async ligado ao quadro.
pub const SEM_SUSPENSO: u32 = u32::MAX;

/// O quadro de um callback chamado várias vezes por uma nativa ([`Motor::repetidor`]).
#[derive(Clone, Copy)]
pub(crate) struct Repetidor {
    qi: usize,
    marca: u32,
    base: u32,
    fim: u32,
    n: u32,
    np: u32,
    proto: u32,
    endereco: u64,
}

/// `repr(C)`: o N2 empurra e lê quadros direto ([`crate::jit`] confere os deslocamentos).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Quadro {
    pub proto: u32,
    pub pc: u32,
    pub base: u32,
    /// Fim dos registradores (`base + registradores`).
    pub fim: u32,
    pub args: Args,
    /// Tamanho da pilha antes do quadro (volta a ele no retorno).
    pub topo: u32,
    pub this: Valor,
    pub funcao: Valor,
    pub new_target: Valor,
    pub contexto: Valor,
    /// Contextos empurrados dentro desta função (para desenrolar exceções).
    pub contextos: u32,
    /// Quadro aberto por `chamar`/`retomar`: o retorno dele sai do laço.
    pub entrada: bool,
    pub construindo: bool,
    pub suspenso: u32,
}

/// Quadro de gerador ou função async parado num `yield`/`await`.
pub struct QuadroSalvo {
    pub quadro: Quadro,
    /// `pilha[topo..fim]`.
    pub valores: Vec<Valor>,
    /// Promessa da função async (undefined em gerador; o objeto, em gerador async).
    pub promessa: Valor,
    /// Por que parou: [`RENDEU`], [`AGUARDA`] ou [`RENDEU_CRU`].
    pub modo: u8,
}

/// Parou num `yield` (o valor vira `{value, done: false}`).
pub const RENDEU: u8 = 0;
/// Parou num `await`.
pub const AGUARDA: u8 = 1;
/// `yield*` de gerador comum: o resultado do iterador de dentro sai como está.
pub const RENDEU_CRU: u8 = 2;

/// Como o `yield`/`yield*` parado foi retomado (lido por `ModoRetomada`/`Delega`).
const RETOMOU_NORMAL: u8 = 0;
const RETOMOU_LANCA: u8 = 1;
const RETOMOU_RETORNA: u8 = 2;

/// Como um laço termina.
#[derive(Clone, Copy, Debug)]
pub enum Fim {
    Retorno(Valor),
    Suspenso(Valor),
}

/// Resultado de um passo do `yield*`.
enum Passo {
    /// O de dentro acabou: valor do `yield*`.
    Fim(Valor),
    /// Retomado com `return` e o de dentro acabou (ou não tem `return`): a função retorna.
    Retorno(Valor),
    /// Para o quadro com o valor, pelo motivo dado.
    Para(Valor, u8),
}

/// Como o quadro do topo recomeça.
#[derive(Clone, Copy, Debug)]
pub enum Entrada {
    Normal(Valor),
    Lanca(Valor),
    Retorna(Valor),
}

/// Estado de iteração rápida sobre array (registro de iterador).
const IT_ARRAY: Valor = Valor::BURACO;

// Layout do contexto: [forma][len][pai][slots…]. `with`: len com o bit 31, slot 0 = objeto.
const CTX_COM: u32 = 1 << 31;

impl Motor {
    // ---------------------------------------------------------------- contextos

    pub fn novo_contexto(&mut self, n: u32, pai: Valor) -> Valor {
        let d = self.g.alocar(12 + 4 * n);
        self.g.por_u32(d, crate::formas::CONTEXTO.0);
        self.g.por_u32(d + 4, n);
        self.g.por_valor(d, d + 8, pai);
        for i in 0..n {
            self.g.por_u32(d + 12 + 4 * i, Valor::UNDEFINED.0);
        }
        Valor::de_deslocamento(d)
    }

    #[inline]
    fn ctx_pai(&self, c: Valor) -> Valor {
        self.g.valor_em(c.deslocamento() + 8)
    }

    #[inline]
    fn ctx_subir(&self, mut c: Valor, n: u32) -> Valor {
        for _ in 0..n {
            c = self.ctx_pai(c);
        }
        c
    }

    #[inline]
    fn ctx_ler(&self, c: Valor, k: u32) -> Valor {
        self.g.valor_em(c.deslocamento() + 12 + 4 * k)
    }

    #[inline]
    fn ctx_gravar(&mut self, c: Valor, k: u32, v: Valor) {
        let d = c.deslocamento();
        self.g.por_valor(d, d + 12 + 4 * k, v);
    }

    // ---------------------------------------------------------------- globais

    fn ler_global(&mut self, c: Chave, typeof_: bool) -> Res<Valor> {
        if let Some(&(i, _)) = self.lexico.get(&c.0) {
            let v = self.lexico_valores[i as usize];
            if v == Valor::BURACO {
                let n = self.nomes.texto_rust(c);
                return self.erro_referencia(&format!("Cannot access '{n}' before initialization"));
            }
            return Ok(v);
        }
        let g = self.intr.global;
        if let Some(d) = self.propria(g, c) {
            if d.attrs & attr::ACESSOR == 0 {
                return Ok(self.ler_campo(g, d.slot));
            }
        }
        let k = ChaveProp::Nome(c);
        if self.tem(g, k)? {
            return self.obter(g, k);
        }
        if typeof_ {
            return Ok(Valor::UNDEFINED);
        }
        let n = self.nomes.texto_rust(c);
        self.erro_referencia(&format!("{n} is not defined"))
    }

    fn gravar_global(&mut self, c: Chave, v: Valor, estrito: bool) -> Res<()> {
        if let Some(&(i, constante)) = self.lexico.get(&c.0) {
            if self.lexico_valores[i as usize] == Valor::BURACO {
                let n = self.nomes.texto_rust(c);
                return self.erro_referencia(&format!("Cannot access '{n}' before initialization"));
            }
            if constante {
                return self.erro_tipo("Assignment to constant variable.");
            }
            self.lexico_valores[i as usize] = v;
            return Ok(());
        }
        let g = self.intr.global;
        let k = ChaveProp::Nome(c);
        if estrito && !self.tem(g, k)? {
            let n = self.nomes.texto_rust(c);
            return self.erro_referencia(&format!("{n} is not defined"));
        }
        self.gravar(g, k, v, estrito)
    }

    /// O objeto de `with` mais próximo, nos `n` primeiros contextos, que tem o nome (HasBinding,
    /// §9.1.1.2.1: `Symbol.unscopables` esconde o nome).
    fn com_que_tem(&mut self, mut ctx: Valor, c: Chave, mut n: u32) -> Res<Option<Valor>> {
        let k = ChaveProp::Nome(c);
        while ctx != Valor::UNDEFINED && n > 0 {
            if self.g.u32_em(ctx.deslocamento() + 4) & CTX_COM != 0 {
                let o = self.ctx_ler(ctx, 0);
                if self.tem(o, k)? {
                    let u = self.obter(o, ChaveProp::Nome(simb::UNSCOPABLES))?;
                    let escondido = if self.e_objeto(u) {
                        let x = self.obter(u, k)?;
                        self.verdadeiro(x)
                    } else {
                        false
                    };
                    if !escondido {
                        return Ok(Some(o));
                    }
                }
            }
            ctx = self.ctx_pai(ctx);
            n -= 1;
        }
        Ok(None)
    }

    /// Busca dinâmica (`with`): os objetos de `with` da corrente, depois o global.
    fn ler_dinamico(&mut self, ctx: Valor, c: Chave, typeof_: bool) -> Res<Valor> {
        match self.com_que_tem(ctx, c, u32::MAX)? {
            Some(o) => self.obter(o, ChaveProp::Nome(c)),
            None => self.ler_global(c, typeof_),
        }
    }

    fn gravar_dinamico(&mut self, ctx: Valor, c: Chave, v: Valor, estrito: bool) -> Res<()> {
        match self.com_que_tem(ctx, c, u32::MAX)? {
            Some(o) => self.gravar(o, ChaveProp::Nome(c), v, estrito),
            None => self.gravar_global(c, v, estrito),
        }
    }

    // ---------------------------------------------------------------- pilha de chamadas

    /// Texto `stack` dos erros: a mensagem é posta por quem mostra; aqui só as linhas "at".
    pub fn texto_da_pilha(&mut self) -> Valor {
        let mut s = String::new();
        for q in self.quadros.iter().rev().take(10) {
            let p = &self.protos[q.proto as usize];
            let fb = p.codigo();
            let pos = fb.posicao_de(q.pc.saturating_sub(1)) as usize;
            let nome = fb.nome.map(|t| String::from_utf16_lossy(&p.script.textos[t as usize])).unwrap_or_else(|| "<anônima>".into());
            let origem = p.origem.as_deref().unwrap_or("");
            // Linha e coluna (a partir de 1) no texto do script, como nos outros motores.
            let local = match p.fonte.as_deref().and_then(|f| f.get(..pos.min(f.len()))) {
                Some(antes) => {
                    let linha = antes.matches('\n').count() + 1;
                    let coluna = antes.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                    format!("{origem}:{linha}:{coluna}")
                }
                None => format!("{origem}:{pos}"),
            };
            s.push_str(&format!("\n    at {nome} ({local})"));
        }
        self.texto(&s)
    }

    /// Erro criado pelo motor no meio de uma instrução (`null.a`, `x is not a function`): a
    /// pilha foi montada com o quadro ainda marcado na última chamada, e a linha saía errada.
    /// Refaz com a instrução `ini`, se o erro é desta função. Só roda quando há erro.
    #[cold]
    fn reanotar_erro(&mut self, e: Valor, qi: usize, ini: usize) {
        let (ultimo, profundidade) = std::mem::replace(&mut self.erro_interno, (Valor::UNDEFINED, 0));
        if ultimo != e || profundidade as usize != self.quadros.len() || !self.e_objeto(e) {
            return;
        }
        let salvo = self.quadros[qi].pc;
        self.quadros[qi].pc = ini as u32 + 1;
        let pilha = self.texto_da_pilha();
        self.quadros[qi].pc = salvo;
        self.definir_propria(e, crate::nomes::fixos::STACK, pilha, crate::formas::attr::METODO);
    }

    // ---------------------------------------------------------------- entrar em funções

    /// Empurra o quadro de uma função de bytecode. `args` aponta para a pilha.
    fn empurrar_quadro(&mut self, f: Valor, this: Valor, args: Args, nt: Valor, construindo: bool) -> Res<()> {
        let proto = self.interno(f, F_CODIGO);
        let p = &self.protos[proto as usize];
        let (np, nregs, fl) = (p.parametros, p.registradores, p.flags);
        if self.quadros.len() > 10_000 {
            return self.erro_alcance("Maximum call stack size exceeded");
        }
        let this = if fl.seta {
            Valor::UNDEFINED
        } else if !fl.estrita && !construindo {
            if this.e_nulo_ou_indefinido() {
                self.intr.global
            } else {
                self.para_objeto(this)?
            }
        } else {
            this
        };
        self.empilhar(f, proto, np, nregs, this, args, nt, construindo);
        Ok(())
    }

    /// Empurra o quadro de `f` (bytecode `proto`) com o `this` já pronto.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn empilhar(&mut self, f: Valor, proto: u32, np: u32, nregs: u32, this: Valor, args: Args, nt: Valor, construindo: bool) {
        let topo = self.pilha.len() as u32;
        let (base, args_q) = if args.n <= np {
            // O caso comum: os argumentos e o resto dos registradores (undefined) de uma vez.
            let fim = topo + nregs.max(np);
            self.pilha.reserve((fim - topo) as usize);
            for i in 0..args.n {
                let v = self.pilha[(args.base + i) as usize];
                self.pilha.push(v);
            }
            self.pilha.extend(std::iter::repeat_n(Valor::UNDEFINED, (fim - topo - args.n) as usize));
            (topo, Args { base: topo, n: args.n })
        } else {
            self.pilha.extend_from_within(args.base as usize..(args.base + args.n) as usize);
            let base = self.pilha.len() as u32;
            self.pilha.extend_from_within(topo as usize..(topo + np) as usize);
            (base, Args { base: topo, n: args.n })
        };
        let fim = base + nregs.max(np);
        self.pilha.resize(fim as usize, Valor::UNDEFINED);
        let contexto = Valor(self.interno(f, F_CONTEXTO));
        self.quadros.push(Quadro {
            proto,
            pc: 0,
            base,
            fim,
            args: args_q,
            topo,
            this,
            funcao: f,
            new_target: nt,
            contexto,
            contextos: 0,
            entrada: false,
            construindo,
            suspenso: SEM_SUSPENSO,
        });
        let p = &mut self.protos[proto as usize];
        p.chamadas = p.chamadas.saturating_add(1);
    }

    /// O caminho curto do `entrar` para o N2: função de bytecode comum (não classe, não
    /// gerador, não async) que já tem código, com `this` que não precisa virar objeto. Falso =
    /// o caminho geral.
    #[inline]
    fn jit_empurrar(&mut self, f: Valor, this: Valor, args: Args) -> Option<u32> {
        if !f.e_ref() || self.quadros.len() > 10_000 {
            return None;
        }
        if self.formas.tipo_classe(self.forma(f)) != [crate::formas::Tipo::Objeto as u8, Classe::Funcao as u8] {
            return None;
        }
        let c = self.interno(f, F_CODIGO);
        if c & BIT_NATIVA != 0 {
            return None;
        }
        let p = &self.protos[c as usize];
        let (np, nregs, fl) = (p.parametros, p.registradores, p.flags);
        if p.jit.is_none() || fl.construtor_classe || fl.assincrona || fl.gerador {
            return None;
        }
        let this = if fl.seta {
            Valor::UNDEFINED
        } else if fl.estrita {
            this
        } else if this.e_nulo_ou_indefinido() {
            self.intr.global
        } else if self.e_objeto(this) {
            this
        } else {
            return None;
        };
        self.empilhar(f, c, np, nregs, this, args, Valor::UNDEFINED, false);
        Some(c)
    }

    /// Chama `f`. `Some(v)`: terminou (nativa); `None`: empurrou um quadro de bytecode.
    pub(crate) fn entrar(&mut self, f: Valor, this: Valor, args: Args) -> Res<Option<Valor>> {
        self.checar_pilha()?;
        if !self.e_objeto(f) {
            return self.nao_e_funcao(f);
        }
        match self.formas.get(self.forma(f)).classe {
            Classe::Funcao => {
                let c = self.interno(f, F_CODIGO);
                if c & BIT_NATIVA != 0 {
                    return self.chamar_nativa(c & !BIT_NATIVA, f, this, args, Valor::UNDEFINED).map(Some);
                }
                let fl = self.protos[c as usize].flags;
                if fl.construtor_classe {
                    return self.erro_tipo("Class constructor cannot be invoked without 'new'");
                }
                // Gerador async começa como gerador: `IniciaGerador` cria o objeto.
                if fl.assincrona && !fl.gerador {
                    return self.iniciar_async(f, this, args).map(Some);
                }
                self.empurrar_quadro(f, this, args, Valor::UNDEFINED, false)?;
                Ok(None)
            }
            Classe::FuncaoLigada => {
                let (alvo, bthis, bargs) = (Valor(self.interno(f, 0)), Valor(self.interno(f, 1)), Valor(self.interno(f, 2)));
                let novos = self.args_ligados(bargs, args);
                let r = self.entrar(alvo, bthis, novos);
                if let Ok(Some(_)) | Err(_) = r {
                    self.pilha.truncate(novos.base as usize);
                }
                r
            }
            Classe::Proxy => {
                let v = self.proxy_chamar(f, this, args)?;
                Ok(Some(v))
            }
            _ => self.nao_e_funcao(f),
        }
    }

    fn nao_e_funcao<T>(&mut self, f: Valor) -> Res<T> {
        let d = self.descrever(f);
        self.erro_tipo(&format!("{d} is not a function"))
    }

    /// Descrição curta de um valor para mensagens.
    pub fn descrever(&mut self, v: Valor) -> String {
        if self.e_texto(v) {
            let s = self.texto_rust(v);
            return format!("\"{}\"", s.chars().take(40).collect::<String>());
        }
        if self.e_objeto(v) {
            return if self.e_funcao(v) { "function".into() } else { "object".into() };
        }
        if self.e_simbolo(v) {
            return "Symbol()".into();
        }
        self.para_texto_rust(v).unwrap_or_default()
    }

    /// Junta os argumentos ligados (lista) com os da chamada, no topo da pilha.
    fn args_ligados(&mut self, bargs: Valor, args: Args) -> Args {
        let base = self.pilha.len() as u32;
        let nb = if bargs == Valor::LISTA_VAZIA { 0 } else { self.lista_len(bargs) };
        for i in 0..nb {
            let v = self.lista_get(bargs, i);
            self.pilha.push(v);
        }
        self.pilha.extend_from_within(args.base as usize..(args.base + args.n) as usize);
        Args { base, n: nb + args.n }
    }

    fn chamar_nativa(&mut self, id: u32, f: Valor, this: Valor, args: Args, nt: Valor) -> Res<Valor> {
        let nf = self.nativas[id as usize].f;
        // Em raízes: a nativa pode coletar ([`Motor::chamar_coletavel`]).
        self.raizes.push(self.chamado);
        self.chamado = f;
        self.aninhamento += 1;
        let r = nf(self, this, args, nt);
        self.aninhamento -= 1;
        self.chamado = self.raizes.pop().unwrap();
        r
    }

    fn proxy_chamar(&mut self, p: Valor, this: Valor, args: Args) -> Res<Valor> {
        let alvo = Valor(self.interno(p, 0));
        let tratador = Valor(self.interno(p, 1));
        let k = self.nomes.internar_str("apply");
        let ap = self.obter_metodo(tratador, ChaveProp::Nome(k))?;
        let lista: Vec<Valor> = (0..args.n).map(|i| self.arg(args, i)).collect();
        match ap {
            Some(f) => {
                let arr = self.array_de(&lista);
                self.chamar(f, tratador, &[alvo, this, arr])
            }
            None => self.chamar(alvo, this, &lista),
        }
    }

    /// `new f(...args)` com `new.target = nt`. `None`: empurrou quadro de bytecode.
    pub(crate) fn construir_no_laco(&mut self, f: Valor, args: Args, nt: Valor) -> Res<Option<Valor>> {
        self.checar_pilha()?;
        if !self.e_construtor(f) {
            let d = self.descrever(f);
            return self.erro_tipo(&format!("{d} is not a constructor"));
        }
        match self.formas.get(self.forma(f)).classe {
            Classe::FuncaoLigada => {
                let (alvo, bargs) = (Valor(self.interno(f, 0)), Valor(self.interno(f, 2)));
                let novos = self.args_ligados(bargs, args);
                let nt = if nt == f { alvo } else { nt };
                let r = self.construir_no_laco(alvo, novos, nt);
                if let Ok(Some(_)) | Err(_) = r {
                    self.pilha.truncate(novos.base as usize);
                }
                r
            }
            Classe::Proxy => {
                let alvo = Valor(self.interno(f, 0));
                let nt = if nt == f { alvo } else { nt };
                let lista: Vec<Valor> = (0..args.n).map(|i| self.arg(args, i)).collect();
                self.construir(alvo, &lista, nt).map(Some)
            }
            _ => {
                let c = self.interno(f, F_CODIGO);
                if c & BIT_NATIVA != 0 {
                    return self.chamar_nativa(c & !BIT_NATIVA, f, Valor::UNDEFINED, args, nt).map(Some);
                }
                let fl = self.protos[c as usize].codigo().flags;
                let this = if fl.classe_derivada {
                    Valor::BURACO
                } else {
                    let op = self.intr.objeto_proto;
                    let o = self.objeto_de_construtor(nt, op)?;
                    if fl.construtor_classe {
                        self.rodar_campos(f, o)?;
                    }
                    o
                };
                self.empurrar_quadro(f, this, args, nt, true)?;
                Ok(None)
            }
        }
    }

    /// Objeto novo com o protótipo de `nt.prototype` (ou `padrao`).
    pub fn objeto_de_construtor(&mut self, nt: Valor, padrao: Valor) -> Res<Valor> {
        let p = self.obter(nt, ChaveProp::Nome(fixos::PROTOTYPE))?;
        let p = if self.e_objeto(p) { p } else { padrao };
        let forma = if p == self.intr.objeto_proto { self.intr.forma_objeto } else { self.forma_para(Classe::Comum, p, 4) };
        Ok(self.novo_objeto(forma))
    }

    /// InitializeInstanceElements: os métodos e acessores privados da classe `f` em `o`, depois
    /// os campos na ordem. A lista é (chave, valor, tipo) por elemento (ver `DefCampo`).
    pub fn rodar_campos(&mut self, f: Valor, o: Valor) -> Res<()> {
        let Some(d) = self.propria(f, simb::CAMPOS) else { return Ok(()) };
        let lista = self.ler_campo(f, d.slot);
        let n = self.lista_len(lista) / 3;
        let item = |m: &Self, i: u32| (m.lista_get(lista, 3 * i), m.lista_get(lista, 3 * i + 1), m.lista_get(lista, 3 * i + 2).como_int() as u32);
        // Objeto que já tem os métodos (o mesmo objeto iniciado duas vezes): TypeError antes de
        // pôr qualquer coisa.
        for i in 0..n {
            let (k, _, t) = item(self, i);
            if matches!(t, 1..=3) && self.privado(o, k).is_some() {
                return self.erro_tipo("Cannot initialize private methods twice on the same object");
            }
        }
        for i in 0..n {
            let (k, v, t) = item(self, i);
            if matches!(t, 1..=3) {
                self.acrescentar_privado(o, k, v, t)?;
            }
        }
        for i in 0..n {
            let (k, init, t) = item(self, i);
            if matches!(t, 1..=3) {
                continue;
            }
            let v = if init == Valor::UNDEFINED { Valor::UNDEFINED } else { self.chamar(init, o, &[])? };
            if t == 4 {
                self.nomear_funcao(v, k, 0)?;
            }
            if self.e_simbolo(k) && self.escondidos.contains(&(self.chave_do_simbolo(k).0 & !Chave::BIT_SIMBOLO)) {
                self.acrescentar_privado(o, k, v, 0)?;
                continue;
            }
            let k = self.para_chave(k)?;
            if !self.criar_dado(o, k, v)? {
                return self.erro_tipo("Cannot define class field");
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- chamadas aninhadas

    fn empurrar_args(&mut self, args: &[Valor]) -> Args {
        let base = self.pilha.len() as u32;
        self.pilha.extend_from_slice(args);
        Args { base, n: args.len() as u32 }
    }

    /// Cada laço aberto por nativa (getter, `toString`, callback de `map`, Proxy) gasta ~1 KB
    /// da pilha do Rust, fora da conta dos 10.000 quadros: 1000 cabe numa thread de 2 MB.
    /// Recursão dentro do Rust (Proxy de Proxy, `bind` de `bind`, JSON fundo) não passa pela
    /// pilha de quadros: confere quanto da pilha do Rust já foi usado desde a entrada.
    #[inline(always)]
    pub(crate) fn checar_pilha(&mut self) -> Res<()> {
        if self.base_pilha != 0 && self.base_pilha.saturating_sub(endereco_pilha()) > self.limite_pilha {
            return self.erro_alcance("Maximum call stack size exceeded");
        }
        Ok(())
    }

    /// Uma volta de laço longo dentro de nativa (`[].join` com length enorme, `indexOf` em
    /// objeto com length 1e15): olha o `interromper` como `SaltaAtras` (senão o botão Parar
    /// não alcança) e o teto da gaiola (lá dentro o coletor não roda).
    #[inline(always)]
    pub(crate) fn tique(&mut self) -> Res<()> {
        self.voltas = self.voltas.wrapping_add(1);
        if self.voltas & 0xFFFF == 0 {
            return self.tique_lento();
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn tique_lento(&mut self) -> Res<()> {
        if let Some(motivo) = self.interromper.as_mut().and_then(|f| f()) {
            return Err(self.erro(crate::intrinsecos::TipoErro::Erro, &motivo));
        }
        if !self.g.cabe(0) && !self.terminando {
            return self.sem_memoria_erro().map(|_| ());
        }
        Ok(())
    }

    /// `n ± 1` de um valor já numérico (`++`/`--`).
    /// O int32 de um número que já é inteiro de 32 bits (inteiro pequeno ou caixa); senão None.
    #[inline]
    fn int32_de(&self, v: Valor) -> Option<i32> {
        if v.e_int() {
            return Some(v.como_int());
        }
        if !self.e_numero(v) {
            return None;
        }
        let x = self.f64_de(v);
        let i = x as i32;
        (f64::from(i) == x).then_some(i)
    }

    fn mais_um(&mut self, n: Valor, mais: bool) -> Res<Valor> {
        let d = if mais { 1 } else { -1 };
        if self.e_bigint(n) {
            let x = self.ler_bigint(n).somar(&crate::grande::Grande::de_i64(i64::from(d)));
            return Ok(self.novo_bigint(&x));
        }
        let n = if n.e_int() { n } else { self.para_numero_valor(n)? };
        self.somar_int(n, d)
    }

    #[inline(always)]
    fn checar_aninhamento(&mut self) -> Res<()> {
        if self.aninhamento > 1000 {
            return self.erro_alcance("Maximum call stack size exceeded");
        }
        Ok(())
    }

    /// Chama `f(this, ...args)` de dentro do motor (nativas, conversões).
    pub fn chamar(&mut self, f: Valor, this: Valor, args: &[Valor]) -> Res<Valor> {
        self.checar_aninhamento()?;
        self.aninhamento += 1;
        let r = match self.jit_chamar_nativa(f, this, args) {
            Some(r) => r,
            None => self.chamar_externo(f, this, args),
        };
        self.aninhamento -= 1;
        r
    }

    /// [`Motor::chamar`] de nativa que guarda em `raizes` tudo o que usa depois da chamada
    /// (inclusive o `this` e os argumentos que recebeu): o coletor pode rodar lá dentro. Conta
    /// como seguros o nível desta chamada e o da nativa logo abaixo.
    pub(crate) fn chamar_coletavel(&mut self, f: Valor, this: Valor, args: &[Valor]) -> Res<Valor> {
        self.seguro += 2;
        let r = self.chamar(f, this, args);
        self.seguro -= 2;
        r
    }

    /// Chamada vinda de fora (o embutidor). Não pode haver `Valor` solto no Rust durante ela
    /// além dos registrados em raízes: o coletor pode rodar.
    pub fn chamar_externo(&mut self, f: Valor, this: Valor, args: &[Valor]) -> Res<Valor> {
        let externo = self.base_pilha == 0;
        if externo {
            self.base_pilha = endereco_pilha();
        }
        let marca = self.pilha.len();
        let a = self.empurrar_args(args);
        let r = match self.entrar(f, this, a) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => {
                self.quadros.last_mut().unwrap().entrada = true;
                match self.jit_externo() {
                    Ok(v) => Ok(v),
                    Err(ent) => match self.laco(ent) {
                        Ok(Fim::Retorno(v)) | Ok(Fim::Suspenso(v)) => Ok(v),
                        Err(e) => Err(e),
                    },
                }
            }
            Err(e) => Err(e),
        };
        self.pilha.truncate(marca);
        if externo {
            self.base_pilha = 0;
        }
        r
    }

    /// N2: a chamada de [`Motor::chamar`] (callback de `map`, comparador de `sort`…) a uma função
    /// de bytecode comum que já tem código: o quadro de entrada empurrado aqui mesmo e o código
    /// rodado direto (o que `chamar_externo` + `entrar` + `jit_externo` fazem, sem o resto).
    /// None = o caminho geral.
    /// N2: a chamada de [`Motor::chamar`] (callback de `map`, comparador de `sort`…) a uma função
    /// de bytecode comum que já tem código (a entrada dela na tabela da chamada direta): o quadro
    /// de entrada empurrado aqui mesmo e o código rodado direto, como o `Chama` do código faz.
    /// None = o caminho geral.
    #[inline]
    fn jit_chamar_nativa(&mut self, f: Valor, this: Valor, args: &[Valor]) -> Option<Res<Valor>> {
        if self.jit_modo == 0 || self.base_pilha == 0 || self.quadros.len() > 10_000 || self.jit_profundidade >= 200 || !f.e_ref() {
            return None;
        }
        if self.formas.tipo_classe(self.forma(f)) != [crate::formas::Tipo::Objeto as u8, Classe::Funcao as u8] {
            return None;
        }
        let c = self.interno(f, F_CODIGO);
        if c & BIT_NATIVA != 0 {
            return None;
        }
        let e = *self.jit_tabela.get(c as usize)?;
        if e.endereco == 0 {
            return None;
        }
        let this = match e.modo_this {
            0 => Valor::UNDEFINED,
            1 => this,
            _ if this.e_nulo_ou_indefinido() => self.intr.global,
            _ if self.e_objeto(this) => this,
            _ => return None,
        };
        if self.checar_pilha().is_err() {
            return None;
        }
        // O quadro como `empilhar`: os argumentos e o resto dos registradores (undefined); com
        // argumentos a mais, todos eles antes (os do `arguments`) e os registradores depois.
        let (marca, n, np) = (self.pilha.len() as u32, args.len() as u32, e.parametros);
        let base = if n <= np { marca } else { marca + n };
        let fim = base + e.tam;
        let total = (fim - marca) as usize;
        let livre = self.pilha.reservar_fim(total);
        let k = n.min(np) as usize;
        // SEGURANÇA: `total` lugares reservados: os n argumentos (se há a mais), os k primeiros
        // e o resto dos `e.tam` registradores, nesta ordem, sem passar do fim.
        let regs = unsafe {
            // Poucos itens: laços simples (a cópia de tamanho variável vira `memcpy`).
            let mut q = livre;
            if n > np {
                for (j, &v) in args.iter().enumerate() {
                    q.add(j).write(v);
                }
                q = q.add(n as usize);
            }
            for (j, &v) in args[..k].iter().enumerate() {
                q.add(j).write(v);
            }
            for j in k..e.tam as usize {
                q.add(j).write(Valor::UNDEFINED);
            }
            self.pilha.crescer_len(total);
            q
        };
        let contexto = Valor(self.interno(f, F_CONTEXTO));
        let livre = self.quadros.reservar_fim(1);
        let quadro = Quadro {
            proto: c,
            pc: 0,
            base,
            fim,
            args: Args { base: marca, n },
            topo: marca,
            this,
            funcao: f,
            new_target: Valor::UNDEFINED,
            contexto,
            contextos: 0,
            entrada: true,
            construindo: false,
            suspenso: SEM_SUSPENSO,
        };
        // SEGURANÇA: um lugar reservado.
        unsafe {
            livre.write(quadro);
            self.quadros.crescer_len(1);
        }
        let qi = self.quadros.len() - 1;
        if self.jit_profundidade == 0 {
            self.jit_limite = endereco_pilha().saturating_sub(256 * 1024);
        }
        self.jit_profundidade += 1;
        let (topo, teto) = self.g.ptr_topo_teto();
        let m: *mut Motor = self;
        let mut ctx = crate::jit::Contexto {
            regs,
            gaiola: self.g.ptr(),
            pendente: std::ptr::addr_of!(self.g.coleta_pendente),
            motor: m,
            acc: Valor::UNDEFINED.0,
            pc: 0,
            voltas: 0,
            motivo: 0,
            formas: self.formas.ptr(),
            n_formas: self.formas.len() as u32,
            classes: self.formas.ptr_tc(),
            limite: self.jit_limite,
            caixa: 0,
            quadro: qi as u32,
            base,
            contexto: contexto.0,
            topo,
            teto,
        };
        // SEGURANÇA: o endereço é de um código vivo (os velhos ficam em `jit_antigos`) com no
        // máximo `e.tam` registradores, todos empurrados acima; o motor só é usado pelas nativas
        // do próprio código enquanto ele roda.
        unsafe { crate::jit::rodar_endereco(e.endereco, &mut ctx) };
        self.jit_profundidade -= 1;
        let r = if ctx.motivo == crate::jit::SAIU_RETORNO && ctx.voltas == 0 {
            // O `Retorna` de um quadro de entrada.
            self.quadros.pop();
            Ok(Valor(ctx.acc))
        } else {
            self.jit_nativa_resto(qi, &ctx)
        };
        self.pilha.truncate(marca as usize);
        Some(r)
    }

    /// N2: prepara chamadas repetidas de `f` com `n` argumentos por uma nativa (o callback de
    /// `forEach`, `map`, `filter`…): confere a função uma vez e deixa o quadro dela empurrado;
    /// cada [`Motor::repetir`] só reescreve os argumentos e roda o código. Entre uma chamada e
    /// outra a nativa não pode rodar JS nem lançar com o quadro ali: [`Motor::soltar`] antes.
    /// None = sem código, ou função que não é comum (o caminho geral, `chamar_coletavel`).
    pub(crate) fn repetidor(&mut self, f: Valor, this: Valor, n: u32) -> Option<Repetidor> {
        if self.jit_modo == 0 || self.base_pilha == 0 || self.quadros.len() > 10_000 || self.jit_profundidade >= 200 || !f.e_ref() {
            return None;
        }
        if self.aninhamento > 1000 || self.checar_pilha().is_err() {
            return None;
        }
        if self.formas.tipo_classe(self.forma(f)) != [crate::formas::Tipo::Objeto as u8, Classe::Funcao as u8] {
            return None;
        }
        let c = self.interno(f, F_CODIGO);
        if c & BIT_NATIVA != 0 {
            return None;
        }
        let e = *self.jit_tabela.get(c as usize)?;
        if e.endereco == 0 {
            return None;
        }
        let this = match e.modo_this {
            0 => Valor::UNDEFINED,
            1 => this,
            _ if this.e_nulo_ou_indefinido() => self.intr.global,
            _ if self.e_objeto(this) => this,
            _ => return None,
        };
        // O quadro como `empilhar`, com os argumentos ainda undefined.
        let marca = self.pilha.len() as u32;
        let base = if n <= e.parametros { marca } else { marca + n };
        let fim = base + e.tam;
        self.pilha.resize(fim as usize, Valor::UNDEFINED);
        let contexto = Valor(self.interno(f, F_CONTEXTO));
        self.quadros.push(Quadro {
            proto: c,
            pc: 0,
            base,
            fim,
            args: Args { base: marca, n },
            topo: marca,
            this,
            funcao: f,
            new_target: Valor::UNDEFINED,
            contexto,
            contextos: 0,
            entrada: true,
            construindo: false,
            suspenso: SEM_SUSPENSO,
        });
        if self.jit_profundidade == 0 {
            self.jit_limite = endereco_pilha().saturating_sub(256 * 1024);
        }
        Some(Repetidor { qi: self.quadros.len() - 1, marca, base, fim, n, np: e.parametros, proto: c, endereco: e.endereco })
    }

    /// Tira o quadro do [`Motor::repetidor`] (se ainda está lá).
    pub(crate) fn soltar(&mut self, r: &mut Option<Repetidor>) {
        if let Some(r) = r.take() {
            debug_assert_eq!(self.quadros.len(), r.qi + 1);
            self.quadros.truncate(r.qi);
            self.pilha.truncate(r.marca as usize);
        }
    }

    /// Uma chamada pelo repetidor `r` (`args.len()` = o `n` dele). Se o código não volta pelo
    /// caminho curto, o quadro segue pelo caminho geral e o repetidor acaba (`r` = None).
    pub(crate) fn repetir(&mut self, r: &mut Option<Repetidor>, args: &[Valor]) -> Res<Valor> {
        let rep = r.as_ref().expect("repetidor solto");
        debug_assert!(self.quadros.len() == rep.qi + 1 && self.pilha.len() == rep.fim as usize && args.len() as u32 == rep.n);
        // O código pode ter sido trocado (recompilado, ou a função desistiu do N2) lá dentro.
        if self.jit_tabela.get(rep.proto as usize).map(|e| e.endereco) != Some(rep.endereco) {
            let (f, this) = (self.quadros[rep.qi].funcao, self.quadros[rep.qi].this);
            self.soltar(r);
            return self.chamar_coletavel(f, this, args);
        }
        let rep = *rep;
        // Os argumentos (todos antes, se há a mais que os parâmetros) e o resto undefined: o
        // código pode ter gravado nos registradores na chamada anterior.
        let k = rep.n.min(rep.np) as usize;
        {
            // Poucos itens: laços simples (a cópia de tamanho variável vira `memcpy`).
            let p = &mut self.pilha[rep.marca as usize..rep.fim as usize];
            let (antes, regs) = p.split_at_mut((rep.base - rep.marca) as usize);
            for (d, &v) in antes.iter_mut().zip(args) {
                *d = v;
            }
            for (j, d) in regs.iter_mut().enumerate() {
                *d = if j < k { args[j] } else { Valor::UNDEFINED };
            }
        }
        let q = &mut self.quadros[rep.qi];
        q.pc = 0;
        let contexto = q.contexto;
        self.seguro += 2;
        self.aninhamento += 1;
        self.jit_profundidade += 1;
        let (topo, teto) = self.g.ptr_topo_teto();
        let m: *mut Motor = self;
        let mut ctx = crate::jit::Contexto {
            regs: self.pilha[rep.base as usize..].as_mut_ptr(),
            gaiola: self.g.ptr(),
            pendente: std::ptr::addr_of!(self.g.coleta_pendente),
            motor: m,
            acc: Valor::UNDEFINED.0,
            pc: 0,
            voltas: 0,
            motivo: 0,
            formas: self.formas.ptr(),
            n_formas: self.formas.len() as u32,
            classes: self.formas.ptr_tc(),
            limite: self.jit_limite,
            caixa: 0,
            quadro: rep.qi as u32,
            base: rep.base,
            contexto: contexto.0,
            topo,
            teto,
        };
        // SEGURANÇA: como em `jit_chamar_nativa` (o quadro e os `tam` registradores estão
        // empurrados, o código é o da tabela).
        unsafe { crate::jit::rodar_endereco(rep.endereco, &mut ctx) };
        self.jit_profundidade -= 1;
        let v = if ctx.motivo == crate::jit::SAIU_RETORNO && ctx.voltas == 0 && self.pilha.len() == rep.fim as usize {
            Ok(Valor(ctx.acc))
        } else {
            *r = None;
            let v = self.jit_nativa_resto(rep.qi, &ctx);
            self.pilha.truncate(rep.marca as usize);
            v
        };
        self.aninhamento -= 1;
        self.seguro -= 2;
        v
    }

    /// O código chamado por [`Motor::jit_chamar_nativa`] saiu de outro jeito: o resto do quadro.
    #[cold]
    #[inline(never)]
    fn jit_nativa_resto(&mut self, qi: usize, ctx: &crate::jit::Contexto) -> Res<Valor> {
        let s = self.jit_motivo(qi, ctx.motivo, ctx.voltas, ctx.pc as usize, Valor(ctx.acc));
        match self.jit_externo_saida(qi, s) {
            Ok(v) => Ok(v),
            Err(ent) => match self.laco(ent) {
                Ok(Fim::Retorno(v)) | Ok(Fim::Suspenso(v)) => Ok(v),
                Err(e) => Err(e),
            },
        }
    }

    /// `new f(...args)` de dentro do motor.
    pub fn construir(&mut self, f: Valor, args: &[Valor], nt: Valor) -> Res<Valor> {
        self.checar_aninhamento()?;
        let marca = self.pilha.len();
        self.aninhamento += 1;
        let a = self.empurrar_args(args);
        let r = match self.construir_no_laco(f, a, nt) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => {
                self.quadros.last_mut().unwrap().entrada = true;
                match self.laco(Entrada::Normal(Valor::UNDEFINED)) {
                    Ok(Fim::Retorno(v)) | Ok(Fim::Suspenso(v)) => Ok(v),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        };
        self.aninhamento -= 1;
        self.pilha.truncate(marca);
        r
    }

    /// Roda o topo de um script carregado (`carregar`) e devolve o valor final.
    pub fn executar(&mut self, proto: u32) -> Res<Valor> {
        let f = self.closure(proto, Valor::UNDEFINED, Valor::UNDEFINED, None);
        let g = self.intr.global;
        self.chamar_externo(f, g, &[])
    }

    /// Analisa, compila e roda um script. Erro de sintaxe vira SyntaxError.
    pub fn avaliar(&mut self, fonte: &str) -> Res<Valor> {
        let proto = self.compilar_script(fonte)?;
        self.executar(proto)
    }

    /// Como [`Self::avaliar`], com o nome da fonte que aparece na pilha dos erros
    /// (`at f (origem:linha:coluna)`).
    pub fn avaliar_com_nome(&mut self, fonte: &str, origem: &str) -> Res<Valor> {
        let proto = self.compilar_script(fonte)?;
        let origem: Rc<str> = Rc::from(origem);
        for p in &mut self.protos[proto as usize..] {
            p.origem = Some(Rc::clone(&origem));
        }
        self.executar(proto)
    }

    /// Texto vindo do JS (`eval`, `new Function`): o embutidor pode recusar (CSP).
    pub(crate) fn compilar_texto(&mut self, fonte: &str) -> Res<u32> {
        if let Some(f) = self.ao_compilar_texto {
            if let Err(msg) = f(self) {
                return Err(self.erro(crate::intrinsecos::TipoErro::Eval, &msg));
            }
        }
        self.compilar_script(fonte)
    }

    pub fn compilar_script(&mut self, fonte: &str) -> Res<u32> {
        let script = match falcao_sintaxe::analisar_script(fonte) {
            Ok((arvore, lexico)) => {
                let esc = falcao_sintaxe::analisar_escopos(&arvore);
                match falcao_bytecode::compilar(&arvore, &esc, &lexico) {
                    Ok(s) => {
                        if let Err(m) = crate::regex::validar_literais(&s) {
                            return Err(self.erro(crate::intrinsecos::TipoErro::Sintaxe, &format!("Invalid regular expression: {m}")));
                        }
                        s
                    }
                    Err(e) => return Err(self.erro(crate::intrinsecos::TipoErro::Sintaxe, &e.mensagem)),
                }
            }
            Err(e) => {
                // Onde está o erro, como na pilha: linha e coluna a partir de 1.
                let antes = fonte.get(..(e.posicao as usize).min(fonte.len())).unwrap_or("");
                let linha = antes.matches('\n').count() + 1;
                let coluna = antes.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                let msg = format!("{} (linha {linha}, coluna {coluna})", e.mensagem);
                return Err(self.erro(crate::intrinsecos::TipoErro::Sintaxe, &msg));
            }
        };
        let base = self.carregar(script);
        let fonte: Rc<str> = Rc::from(fonte);
        for p in &mut self.protos[base as usize..] {
            p.fonte = Some(Rc::clone(&fonte));
        }
        Ok(base)
    }

    // ---------------------------------------------------------------- geradores e async

    fn novo_suspenso(&mut self) -> u32 {
        if let Some(i) = self.suspensos_livres.pop() {
            return i;
        }
        self.suspensos.push(None);
        self.suspensos.len() as u32 - 1
    }

    pub(crate) fn liberar_suspenso(&mut self, id: u32) {
        self.suspensos[id as usize] = None;
        self.suspensos_livres.push(id);
    }

    /// Tira o quadro do topo e guarda (para `yield`/`await`).
    fn salvar_quadro(&mut self, pc: u32) -> QuadroSalvo {
        let mut q = self.quadros.pop().unwrap();
        q.pc = pc;
        let valores = self.pilha[q.topo as usize..q.fim as usize].to_vec();
        self.pilha.truncate(q.topo as usize);
        QuadroSalvo { quadro: q, valores, promessa: Valor::UNDEFINED, modo: RENDEU }
    }

    /// Continua um gerador/async parado.
    pub(crate) fn retomar(&mut self, id: u32, ent: Entrada) -> Res<Fim> {
        self.aninhamento += 1;
        let r = self.retomar_no_topo(id, ent);
        self.aninhamento -= 1;
        r
    }

    /// `retomar` sem aninhar: o chamador não segura `Valor` no Rust (a fila de microtarefas),
    /// então o coletor pode rodar dentro.
    pub(crate) fn retomar_no_topo(&mut self, id: u32, ent: Entrada) -> Res<Fim> {
        let Some(salvo) = self.suspensos[id as usize].take() else {
            return self.erro_tipo("Generator is already running");
        };
        let mut q = salvo.quadro;
        let novo = self.pilha.len() as u32;
        let desloc = novo.wrapping_sub(q.topo);
        q.topo = novo;
        q.base = q.base.wrapping_add(desloc);
        q.fim = q.fim.wrapping_add(desloc);
        q.args.base = q.args.base.wrapping_add(desloc);
        q.entrada = true;
        self.pilha.extend_from_slice(&salvo.valores);
        self.quadros.push(q);
        // A promessa da async fica guardada fora enquanto roda.
        self.promessas_async.insert(id, salvo.promessa);
        self.laco(ent)
    }

    fn iniciar_async(&mut self, f: Valor, this: Valor, args: Args) -> Res<Valor> {
        let p = self.nova_promessa();
        let id = self.novo_suspenso();
        self.empurrar_quadro(f, this, args, Valor::UNDEFINED, false)?;
        {
            let q = self.quadros.last_mut().unwrap();
            q.entrada = true;
            q.suspenso = id;
        }
        self.promessas_async.insert(id, p);
        self.aninhamento += 1;
        let r = self.laco(Entrada::Normal(Valor::UNDEFINED));
        self.aninhamento -= 1;
        self.depois_async(id, r);
        Ok(p)
    }

    /// Depois que uma async parou: resolve, rejeita ou espera a promessa do `await`.
    pub(crate) fn depois_async(&mut self, id: u32, r: Res<Fim>) {
        let p = self.promessas_async.remove(&id).unwrap_or(Valor::UNDEFINED);
        if self.e_gerador_async(p) {
            return self.depois_gerador_async(p, id, r);
        }
        match r {
            Ok(Fim::Retorno(v)) => {
                self.liberar_suspenso(id);
                self.resolver_promessa(p, v);
            }
            Err(e) => {
                self.liberar_suspenso(id);
                self.rejeitar_promessa(p, e);
            }
            Ok(Fim::Suspenso(v)) => {
                if let Some(s) = self.suspensos[id as usize].as_mut() {
                    s.promessa = p;
                }
                self.aguardar(id, v);
            }
        }
    }

    /// Await(v) da async/gerador async `id` (já guardado): PromiseResolve pode lançar (o
    /// `constructor` é lido), e aí o erro volta na hora para dentro da função.
    pub(crate) fn aguardar(&mut self, id: u32, v: Valor) {
        match self.promessa_resolve(v) {
            Ok(espera) => self.reagir_async(espera, id),
            Err(e) => {
                let r = self.retomar(id, Entrada::Lanca(e));
                self.depois_async(id, r);
            }
        }
    }

    // ---------------------------------------------------------------- exceções

    /// Procura tratador para `e`. `Ok(acc)`: achou (o quadro do topo segue no tratador);
    /// `Err(e)`: chegou num quadro de entrada sem tratador (já tirado da pilha).
    fn desenrolar(&mut self, e: Valor, mut exato: bool) -> Result<Valor, Valor> {
        loop {
            let Some(q) = self.quadros.last_mut() else { return Err(e) };
            let pc = if exato { q.pc } else { q.pc.wrapping_sub(1) };
            exato = false;
            let p = &self.protos[q.proto as usize];
            let fb = &p.script.funcoes[p.indice as usize];
            let tratador = if self.terminando { None } else { fb.excecoes.iter().find(|x| x.inicio <= pc && pc < x.fim).copied() };
            if let Some(x) = tratador {
                let q = self.quadros.last_mut().unwrap();
                let mut c = q.contexto;
                let mut n = q.contextos;
                while n > x.contextos {
                    c = self.g.valor_em(c.deslocamento() + 8);
                    n -= 1;
                }
                let q = self.quadros.last_mut().unwrap();
                q.contexto = c;
                q.contextos = n;
                q.pc = x.alvo;
                return Ok(e);
            }
            let q = self.quadros.pop().unwrap();
            self.pilha.truncate(q.topo as usize);
            if q.entrada {
                if self.aninhamento == 0 {
                    self.terminando = false;
                }
                return Err(e);
            }
        }
    }

    // ---------------------------------------------------------------- o laço

    /// Roda o quadro do topo até o quadro de entrada voltar (ou parar num yield/await).
    #[allow(unused_assignments)]
    pub(crate) fn laco(&mut self, ent: Entrada) -> Res<Fim> {
        let mut acc = Valor::UNDEFINED;
        let mut pendente: Option<(Valor, bool)> = None;
        // `yield` (ModoRetomada) e `yield*` (Delega) tratam o throw/return eles mesmos.
        let mut retomada = RETOMOU_NORMAL;
        match ent {
            Entrada::Normal(v) => acc = v,
            Entrada::Lanca(e) => {
                if self.parado_em(&[Op::Delega]) {
                    acc = e;
                    retomada = RETOMOU_LANCA;
                } else {
                    pendente = Some((e, false));
                }
            }
            Entrada::Retorna(v) => {
                if self.parado_em(&[Op::Delega, Op::ModoRetomada]) {
                    acc = v;
                    retomada = RETOMOU_RETORNA;
                } else {
                    // Gerador que nem começou: acaba sem rodar nada.
                    let q = self.quadros.pop().unwrap();
                    self.pilha.truncate(q.topo as usize);
                    return Ok(Fim::Retorno(v));
                }
            }
        }
        'quadro: loop {
            if let Some((e, exato)) = pendente.take() {
                match self.desenrolar(e, exato) {
                    Ok(v) => acc = v,
                    Err(e) => return Err(e),
                }
            }
            // Ponto seguro das chamadas e retornos (o das voltas de laço fica em `SaltaAtras`).
            if self.g.coleta_pendente && self.pode_coletar() {
                acc = self.ponto_seguro(acc);
            }
            if self.sem_memoria(&mut acc) {
                pendente = Some((self.sem_memoria_erro().unwrap_err(), false));
                continue 'quadro;
            }
            let qi = self.quadros.len() - 1;
            let q = self.quadros[qi];
            let proto = q.proto as usize;
            let script = Rc::clone(&self.protos[proto].script);
            let fb = &script.funcoes[self.protos[proto].indice as usize];
            let codigo: &[u8] = &fb.codigo;
            let estrito = fb.flags.estrita;
            let base = q.base as usize;
            let mut pc = q.pc as usize;

            macro_rules! reg {
                ($r:expr) => {
                    self.pilha[base + $r as usize]
                };
            }
            // Feedback de tipos do caminho lento; cache estável só custa ler um byte.
            macro_rules! anotar {
                ($c:expr, $a:expr, $b:expr, $r:expr) => {
                    if self.protos[proto].tipos[$c as usize] < tipo::ESTAVEL {
                        self.anotar(proto, $c, $a, $b, $r)
                    }
                };
            }

            // N2: função quente (ou de volta de uma chamada) com código entra nele aqui. Não
            // reentra onde o código acabou de sair para o N1 (a instrução roda no intérprete).
            if self.jit_modo != 0 && self.jit_entrada(proto) {
                let pular = self.jit_pular.take() == Some((qi as u32, pc as u32));
                if !pular {
                    if let Some(cod) = self.jit_de(proto, true) {
                        if cod.entra_em(pc) {
                            match self.rodar_jit(cod, qi, pc, acc) {
                                SaidaJit::Segue(p, a) | SaidaJit::Retorno(p, a) => {
                                    pc = p;
                                    acc = a;
                                }
                                SaidaJit::Aborto => {
                                    acc = self.jit_acc;
                                    if let Some(e) = self.jit_erro.take() {
                                        pendente = Some((e, true));
                                    }
                                    continue 'quadro;
                                }
                            }
                        }
                    }
                }
            }
            let erro: Valor = 'exec: loop {
                let ini = pc;
                let mut escala = 1usize;
                let mut b = codigo[pc];
                pc += 1;
                if b == Op::Largo as u8 {
                    escala = 2;
                    b = codigo[pc];
                    pc += 1;
                } else if b == Op::Largo4 as u8 {
                    escala = 4;
                    b = codigo[pc];
                    pc += 1;
                }
                let op = Op::de_byte(b).expect("byte de instrução inválido");
                macro_rules! u {
                    () => {{
                        if escala == 1 {
                            pc += 1;
                            u32::from(codigo[pc - 1])
                        } else {
                            let (v, l) = ler_operando(codigo, pc, Operando::R, escala);
                            pc += l;
                            v
                        }
                    }};
                }
                macro_rules! i4 {
                    () => {{
                        // Uma checagem de limite e uma leitura, não quatro.
                        let v = i32::from_le_bytes(codigo[pc..pc + 4].try_into().unwrap());
                        pc += 4;
                        v
                    }};
                }
                macro_rules! t {
                    ($e:expr) => {
                        match $e {
                            Ok(x) => x,
                            Err(e) => {
                                self.quadros[qi].pc = ini as u32;
                                self.reanotar_erro(e, qi, ini);
                                break 'exec e;
                            }
                        }
                    };
                }
                macro_rules! nome {
                    ($k:expr) => {
                        match self.protos[proto].consts[$k as usize] {
                            ConstRt::Nome(c) => c,
                            _ => panic!("constante não é nome"),
                        }
                    };
                }
                macro_rules! salvar_pc {
                    () => {
                        self.quadros[qi].pc = pc as u32;
                    };
                }
                // O resto do `SaltaAtras` (já com `pc` no alvo): perfil, interrupção, coleta.
                macro_rules! volta {
                    () => {
                        self.voltas = self.voltas.wrapping_add(1);
                        // Calor por amostra: a função que roda na volta 256ª leva as 256.
                        if self.voltas & 0xFF == 0 {
                            let p = &mut self.protos[proto];
                            p.voltas = p.voltas.saturating_add(256);
                        }
                        if self.voltas & 0xFFFF == 0 {
                            if let Some(motivo) = self.interromper.as_mut().and_then(|f| f()) {
                                let e = self.erro(crate::intrinsecos::TipoErro::Erro, &motivo);
                                t!(Err::<Valor, Valor>(e));
                            }
                        }
                        if self.g.coleta_pendente && self.pode_coletar() {
                            salvar_pc!();
                            acc = self.ponto_seguro(acc);
                        }
                        if self.sem_memoria(&mut acc) {
                            t!(self.sem_memoria_erro());
                        }
                    };
                }
                // N2: no começo do laço, o código de máquina da função (se há) roda as próximas
                // voltas. Função sem código só procura a cada 256 voltas (a amostra do calor).
                macro_rules! osr {
                    () => {
                        if self.jit_modo != 0
                            && (self.jit_modo == 2 || self.voltas & 0xFF == 0 || self.protos[proto].jit.as_ref().is_some_and(|c| c.entra_em(pc)))
                        {
                            if let Some(cod) = self.jit_laco(proto, pc) {
                                {
                                    match self.rodar_jit(cod, qi, pc, acc) {
                                        SaidaJit::Segue(p, a) | SaidaJit::Retorno(p, a) => {
                                            pc = p;
                                            acc = a;
                                        }
                                        SaidaJit::Aborto => {
                                            acc = self.jit_acc;
                                            if let Some(e) = self.jit_erro.take() {
                                                pendente = Some((e, true));
                                            }
                                            continue 'quadro;
                                        }
                                    }
                                }
                            }
                        }
                    };
                }
                // Comparação de inteiros já feita; se a próxima instrução é um salto condicional
                // (a condição de todo laço), salta aqui mesmo.
                macro_rules! comparou {
                    ($sim:expr) => {
                        let sim = $sim;
                        acc = Valor::bool(sim);
                        let prox = codigo.get(pc).copied();
                        if prox == Some(Op::SaltaSeFalso as u8) || prox == Some(Op::SaltaSeVerdadeiro as u8) {
                            let alvo = i32::from_le_bytes(codigo[pc + 1..pc + 5].try_into().unwrap()) as u32 as usize;
                            pc = if sim == (prox == Some(Op::SaltaSeVerdadeiro as u8)) { alvo } else { pc + 5 };
                        }
                    };
                }
                // Chamada: Some(v) = acc; None = quadro novo.
                macro_rules! depois_de_chamar {
                    ($r:expr) => {
                        match $r {
                            Ok(Some(v)) => acc = v,
                            Ok(None) => continue 'quadro,
                            Err(e) => {
                                self.quadros[qi].pc = ini as u32;
                                // A chamada pode ter deixado argumentos ligados na pilha.
                                self.pilha.truncate(self.quadros[qi].fim as usize);
                                break 'exec e;
                            }
                        }
                    };
                }

                match op {
                    Op::Largo | Op::Largo4 => unreachable!(),
                    Op::CarregaUndef => acc = Valor::UNDEFINED,
                    Op::CarregaNulo => acc = Valor::NULL,
                    Op::CarregaVerdadeiro => acc = Valor::TRUE,
                    Op::CarregaFalso => acc = Valor::FALSE,
                    Op::CarregaBuraco => acc = Valor::BURACO,
                    Op::CarregaInt => {
                        let y = i4!();
                        acc = Valor::int(y);
                        // `i < N`: carregar a constante, comparar com o registrador e saltar de uma vez.
                        let o = codigo.get(pc).copied().unwrap_or(0);
                        if (Op::Menor as u8..=Op::MaiorIgual as u8).contains(&o) {
                            let a = reg!(codigo[pc + 1]);
                            if a.e_int() {
                                let x = a.como_int();
                                pc += 3;
                                comparou!(match o {
                                    o if o == Op::Menor as u8 => x < y,
                                    o if o == Op::Maior as u8 => x > y,
                                    o if o == Op::MenorIgual as u8 => x <= y,
                                    _ => x >= y,
                                });
                            }
                        }
                    }
                    Op::CarregaConst => {
                        let k = u!();
                        acc = t!(self.constante(proto, k));
                    }
                    Op::Carrega => acc = reg!(u!()),
                    Op::Guarda => {
                        let r = u!();
                        reg!(r) = acc;
                    }
                    Op::Move => {
                        let (a, b) = (u!(), u!());
                        reg!(a) = reg!(b);
                    }
                    Op::CarregaThis => {
                        let q = self.quadros[qi];
                        let fl = fb.flags;
                        acc = if fl.seta { Valor(self.interno(q.funcao, F_THIS)) } else { q.this };
                        if (fl.seta || fl.classe_derivada) && self.celula_this(acc) {
                            acc = self.lista_get(acc, 0);
                        }
                        if acc == Valor::BURACO {
                            t!(self.erro_referencia::<()>("Must call super constructor in derived class before accessing 'this' or returning from derived constructor"));
                        }
                    }
                    Op::CarregaNewTarget => acc = self.quadros[qi].new_target,
                    Op::CarregaFuncao => acc = self.quadros[qi].funcao,

                    // ---- nomes ----
                    Op::LeGlobal | Op::LeGlobalTypeof => {
                        let (k, ci) = (u!(), u!());
                        let c = nome!(k);
                        acc = t!(self.ler_global_cache(proto, ci, c, op == Op::LeGlobalTypeof));
                    }
                    Op::GravaGlobal => {
                        let (k, ci) = (u!(), u!());
                        let c = nome!(k);
                        t!(self.gravar_global_cache(proto, ci, c, acc, estrito));
                    }
                    Op::DeclaraGlobal => {
                        let (k, n) = (u!(), u!());
                        let c = nome!(k);
                        let g = self.intr.global;
                        if n == 1 {
                            self.definir_propria(g, c, acc, attr::PADRAO);
                        } else if self.propria(g, c).is_none() {
                            self.definir_propria(g, c, Valor::UNDEFINED, attr::PADRAO);
                        }
                    }
                    Op::IniciaGlobal => {
                        let c = nome!(u!());
                        if let Some(&(i, _)) = self.lexico.get(&c.0) {
                            self.lexico_valores[i as usize] = acc;
                        } else {
                            let g = self.intr.global;
                            t!(self.gravar(g, ChaveProp::Nome(c), acc, false));
                        }
                    }
                    Op::DeclaraGlobalLexico => {
                        let (k, n) = (u!(), u!());
                        let c = nome!(k);
                        if !self.lexico.contains_key(&c.0) {
                            let i = self.lexico_valores.len() as u32;
                            self.lexico_valores.push(Valor::BURACO);
                            self.lexico.insert(c.0, (i, n == 1));
                            self.epoca_lexico = self.epoca_lexico.wrapping_add(1);
                        }
                    }
                    Op::LeContexto => {
                        let (n, k) = (u!(), u!());
                        let c = self.ctx_subir(self.quadros[qi].contexto, n);
                        acc = self.ctx_ler(c, k);
                    }
                    Op::LeContextoChecado => {
                        let (n, k) = (u!(), u!());
                        let c = self.ctx_subir(self.quadros[qi].contexto, n);
                        acc = self.ctx_ler(c, k);
                        if acc == Valor::BURACO {
                            t!(self.erro_referencia::<()>("Cannot access variable before initialization"));
                        }
                    }
                    Op::GravaContexto => {
                        let (n, k) = (u!(), u!());
                        let c = self.ctx_subir(self.quadros[qi].contexto, n);
                        self.ctx_gravar(c, k, acc);
                    }
                    Op::GravaConst => {
                        let _ = u!();
                        t!(self.erro_tipo::<()>("Assignment to constant variable."));
                    }
                    Op::EmpurraContexto => {
                        let n = u!();
                        let pai = self.quadros[qi].contexto;
                        let c = self.novo_contexto(n, pai);
                        let q = &mut self.quadros[qi];
                        q.contexto = c;
                        q.contextos += 1;
                    }
                    Op::SaiContexto => {
                        let c = self.quadros[qi].contexto;
                        let pai = self.ctx_pai(c);
                        let q = &mut self.quadros[qi];
                        q.contexto = pai;
                        q.contextos -= 1;
                    }
                    Op::EmpurraCom => {
                        let o = t!(self.para_objeto(acc));
                        let pai = self.quadros[qi].contexto;
                        let c = self.novo_contexto(1, pai);
                        self.g.por_u32(c.deslocamento() + 4, 1 | CTX_COM);
                        self.ctx_gravar(c, 0, o);
                        let q = &mut self.quadros[qi];
                        q.contexto = c;
                        q.contextos += 1;
                    }
                    Op::RenovaContexto => {
                        let c = self.quadros[qi].contexto;
                        let n = self.g.u32_em(c.deslocamento() + 4);
                        let pai = self.ctx_pai(c);
                        let novo = self.novo_contexto(n, pai);
                        for i in 0..n {
                            let v = self.ctx_ler(c, i);
                            self.ctx_gravar(novo, i, v);
                        }
                        self.quadros[qi].contexto = novo;
                    }
                    Op::LeDinamico | Op::LeDinamicoTypeof => {
                        let (k, _ci) = (u!(), u!());
                        let c = nome!(k);
                        let ctx = self.quadros[qi].contexto;
                        acc = t!(self.ler_dinamico(ctx, c, op == Op::LeDinamicoTypeof));
                    }
                    Op::GravaDinamico => {
                        let (k, _ci) = (u!(), u!());
                        let c = nome!(k);
                        let ctx = self.quadros[qi].contexto;
                        t!(self.gravar_dinamico(ctx, c, acc, estrito));
                    }
                    Op::TemCom => {
                        let (k, n) = (u!(), u!());
                        let c = nome!(k);
                        let ctx = self.quadros[qi].contexto;
                        let n = if n == 0xffff { u32::MAX } else { n };
                        acc = Valor::bool(t!(self.com_que_tem(ctx, c, n)).is_some());
                    }
                    Op::ChecaTdz => {
                        let k = u!();
                        if acc == Valor::BURACO {
                            let n = self.nomes.texto_rust(nome!(k));
                            t!(self.erro_referencia::<()>(&format!("Cannot access '{n}' before initialization")));
                        }
                    }

                    // ---- propriedades ----
                    Op::LeProp | Op::LePropAcc => {
                        let o = if op == Op::LeProp { reg!(u!()) } else { acc };
                        let (k, ci) = (u!(), u!());
                        acc = t!(self.ler_prop_cache(proto, ci, o, nome!(k)));
                    }
                    Op::GravaProp => {
                        let (r, k, ci) = (u!(), u!(), u!());
                        let o = reg!(r);
                        t!(self.gravar_prop_cache(proto, ci, o, nome!(k), acc, estrito));
                    }
                    Op::LeItem => {
                        let (r, _ci) = (u!(), u!());
                        let o = reg!(r);
                        acc = t!(self.op_le_item(o, acc));
                    }
                    Op::GravaItem => {
                        let (r0, r1, _ci) = (u!(), u!(), u!());
                        let (o, kv) = (reg!(r0), reg!(r1));
                        t!(self.op_grava_item(o, kv, acc, estrito));
                    }
                    Op::ApagaProp => {
                        let (r, k) = (u!(), u!());
                        let o = t!(self.para_objeto(reg!(r)));
                        let ok = t!(self.apagar(o, ChaveProp::Nome(nome!(k))));
                        if !ok && estrito {
                            t!(self.erro_tipo::<()>("Cannot delete property"));
                        }
                        acc = Valor::bool(ok);
                    }
                    Op::ApagaItem => {
                        let r = u!();
                        let o = t!(self.para_objeto(reg!(r)));
                        let k = t!(self.para_chave(acc));
                        let ok = t!(self.apagar(o, k));
                        if !ok && estrito {
                            t!(self.erro_tipo::<()>("Cannot delete property"));
                        }
                        acc = Valor::bool(ok);
                    }
                    Op::LeSuper | Op::LeSuperItem => {
                        let r = u!();
                        let this = reg!(r);
                        let k = if op == Op::LeSuper { ChaveProp::Nome(nome!(u!())) } else { t!(self.para_chave(reg!(u!()))) };
                        let p = t!(self.super_base(qi));
                        acc = if p == Valor::NULL { Valor::UNDEFINED } else { t!(self.obter_com(p, k, this)) };
                    }
                    Op::GravaSuper => {
                        let (r, k) = (u!(), u!());
                        let this = reg!(r);
                        let p = t!(self.super_base(qi));
                        let ok = t!(self.gravar_com(p, ChaveProp::Nome(nome!(k)), acc, this));
                        if !ok && estrito {
                            t!(self.erro_tipo::<()>("Cannot assign to read only property"));
                        }
                    }
                    Op::GravaSuperItem => {
                        let (r0, r1, r2) = (u!(), u!(), u!());
                        let (this, kv, v) = (reg!(r0), reg!(r1), reg!(r2));
                        let k = t!(self.para_chave(kv));
                        let p = t!(self.super_base(qi));
                        t!(self.gravar_com(p, k, v, this));
                    }
                    Op::LePrivado => {
                        let (r0, r1) = (u!(), u!());
                        acc = t!(self.ler_privado(reg!(r0), reg!(r1)));
                    }
                    Op::GravaPrivado => {
                        let (r0, r1) = (u!(), u!());
                        t!(self.gravar_privado(reg!(r0), reg!(r1), acc));
                    }
                    Op::TemPrivado => {
                        let c = self.chave_do_simbolo(reg!(u!()));
                        if !self.e_objeto(acc) {
                            t!(self.erro_tipo::<()>("Cannot use 'in' operator to search for a private field in a non-object"));
                        }
                        acc = Valor::bool(self.propria(acc, c).is_some());
                    }
                    Op::DefinePrivado => {
                        let (r0, r1, n) = (u!(), u!(), u!());
                        t!(self.acrescentar_privado(reg!(r0), reg!(r1), acc, n));
                    }
                    Op::NovoPrivado => {
                        let d = t!(self.constante(proto, u!()));
                        let s = self.novo_simbolo(d);
                        let k = self.chave_do_simbolo(s);
                        self.escondidos.insert(k.0 & !Chave::BIT_SIMBOLO);
                        acc = s;
                    }

                    // ---- literais ----
                    Op::CriaObjeto => {
                        let n = u!();
                        acc = self.op_cria_objeto(n);
                    }
                    Op::CriaArray => {
                        let _ = u!();
                        let f = self.intr.forma_array;
                        acc = self.novo_objeto(f);
                    }
                    Op::DefProp => {
                        let (r, k, ci) = (u!(), u!(), u!());
                        let (o, c) = (reg!(r), nome!(k));
                        self.op_def_prop(proto, ci, o, c, acc);
                    }
                    Op::DefItem => {
                        let (r0, r1) = (u!(), u!());
                        let k = t!(self.para_chave(reg!(r1)));
                        // CreateDataPropertyOrThrow: `static ['prototype'] = 1` (não configurável) é TypeError.
                        if !t!(self.criar_dado(reg!(r0), k, acc)) {
                            t!(self.erro_tipo::<()>("Cannot redefine property"));
                        }
                    }
                    Op::Empurra => {
                        let a = reg!(u!());
                        self.op_empurra(a, acc);
                    }
                    Op::EmpurraBuraco => {
                        let a = reg!(u!());
                        let n = self.array_len(a);
                        self.definir_array_len(a, n + 1);
                    }
                    Op::EspalhaArray => {
                        let r = u!();
                        let itens = t!(self.coletar_iteravel(acc));
                        let a = reg!(r);
                        for v in itens {
                            let n = self.array_len(a);
                            self.gravar_elemento(a, n, v);
                        }
                    }
                    Op::RestoObjeto => {
                        let (r0, r1, n) = (u!(), u!(), u!());
                        let src = reg!(r0);
                        let mut fora = Vec::new();
                        for i in 0..n {
                            fora.push(t!(self.para_chave(reg!(r1 + i))));
                        }
                        let dest = self.objeto_vazio();
                        t!(self.copiar_proprias(dest, src, &fora));
                        acc = dest;
                    }
                    Op::EspalhaObjeto => {
                        let r = u!();
                        if !acc.e_nulo_ou_indefinido() {
                            let src = t!(self.para_objeto(acc));
                            t!(self.copiar_proprias(reg!(r), src, &[]));
                        }
                    }
                    Op::DefAcessor => {
                        let (r0, r1, n) = (u!(), u!(), u!());
                        let k = t!(self.para_chave(reg!(r1)));
                        self.definir_acessor(reg!(r0), k, acc, n & 1 == 1, n & 2 == 0);
                    }
                    Op::DefMetodo => {
                        let (r0, r1) = (u!(), u!());
                        let k = t!(self.para_chave(reg!(r1)));
                        self.definir_dado(reg!(r0), k, acc, attr::METODO);
                    }
                    Op::DefProto => {
                        let r = u!();
                        if self.e_objeto(acc) || acc == Valor::NULL {
                            self.definir_proto(reg!(r), acc);
                        }
                    }
                    Op::CriaRegex => {
                        let (kc, kf) = (u!(), u!());
                        let corpo = t!(self.constante(proto, kc));
                        let flags = t!(self.constante(proto, kf));
                        acc = t!(self.criar_regex(corpo, flags));
                    }
                    Op::ObjetoTemplate => {
                        let k = u!();
                        acc = self.objeto_template(proto, k);
                    }
                    Op::CriaFuncao => {
                        let k = u!();
                        acc = self.op_cria_funcao(qi, proto, k);
                    }
                    Op::CriaClasse => {
                        let (k, rh) = (u!(), u!());
                        let ctx = self.quadros[qi].contexto;
                        acc = t!(self.criar_classe(proto, k, reg!(rh), ctx));
                    }
                    Op::DefCampo => {
                        let (r0, r1, n) = (u!(), u!(), u!());
                        let (classe, kv) = (reg!(r0), reg!(r1));
                        self.registrar_campo(classe, kv, acc, n);
                    }
                    Op::NomeiaFuncao => {
                        let (r, n) = (u!(), u!());
                        t!(self.nomear_funcao(acc, reg!(r), n));
                    }
                    Op::DefineCasa => {
                        let r = u!();
                        let casa = reg!(r);
                        if self.e_funcao(acc) && self.formas.get(self.forma(acc)).classe == Classe::Funcao {
                            self.por_interno_valor(acc, F_CASA, casa);
                        }
                    }

                    // ---- operadores ----
                    Op::Soma => {
                        let (r, c) = (u!(), u!());
                        let (a, b) = (reg!(r), acc);
                        acc = match (a.e_int() && b.e_int()).then(|| a.como_int().checked_add(b.como_int())).flatten().and_then(|s| Valor::int_se_cabe(i64::from(s))) {
                            Some(v) => {
                                // Inteiro pequeno já é int32: o `| 0` seguinte não faz nada.
                                if codigo.get(pc) == Some(&(Op::ParaInt32 as u8)) {
                                    pc += 2;
                                }
                                v
                            }
                            None => {
                                // `(a + b) | 0` com int32: a soma de dois int32 é exata no f64, então
                                // o ToInt32 dela é a soma com volta. Pula o ParaInt32 seguinte e não
                                // cria o número intermediário.
                                if codigo.get(pc) == Some(&(Op::ParaInt32 as u8)) {
                                    if let (Some(x), Some(y)) = (self.int32_de(a), self.int32_de(b)) {
                                        anotar!(c, a, b, Valor::int(0));
                                        acc = self.numero(f64::from(x.wrapping_add(y)));
                                        pc += 2;
                                        continue 'exec;
                                    }
                                }
                                let v = t!(self.somar(a, b));
                                anotar!(c, a, b, v);
                                v
                            }
                        };
                    }
                    Op::Sub | Op::Mul | Op::Div | Op::Resto | Op::Potencia => {
                        let (r, c) = (u!(), u!());
                        let ar = match op {
                            Op::Sub => Arit::Sub,
                            Op::Mul => Arit::Mul,
                            Op::Div => Arit::Div,
                            Op::Resto => Arit::Resto,
                            _ => Arit::Potencia,
                        };
                        let (a, b) = (reg!(r), acc);
                        acc = t!(self.aritmetica(ar, a, b));
                        anotar!(c, a, b, acc);
                    }
                    Op::E | Op::Ou | Op::OuExclusivo | Op::DeslEsq | Op::DeslDir | Op::DeslDirSemSinal => {
                        let (r, c) = (u!(), u!());
                        let b = match op {
                            Op::E => Bits::E,
                            Op::Ou => Bits::Ou,
                            Op::OuExclusivo => Bits::Xor,
                            Op::DeslEsq => Bits::Esq,
                            Op::DeslDir => Bits::Dir,
                            _ => Bits::DirSemSinal,
                        };
                        let (x, y) = (reg!(r), acc);
                        acc = t!(self.bits(b, x, y));
                        anotar!(c, x, y, acc);
                    }
                    Op::Igual | Op::Diferente => {
                        let (r, c) = (u!(), u!());
                        anotar!(c, reg!(r), acc, Valor::int(0));
                        let i = t!(self.iguais(reg!(r), acc));
                        acc = Valor::bool(i == (op == Op::Igual));
                    }
                    Op::Identico | Op::NaoIdentico => {
                        let r = u!();
                        let i = self.identicos(reg!(r), acc);
                        acc = Valor::bool(i == (op == Op::Identico));
                    }
                    Op::Menor | Op::Maior | Op::MenorIgual | Op::MaiorIgual => {
                        let (r, c) = (u!(), u!());
                        let (a, b) = (reg!(r), acc);
                        if a.e_int() && b.e_int() {
                            let (x, y) = (a.como_int(), b.como_int());
                            comparou!(match op {
                                Op::Menor => x < y,
                                Op::Maior => x > y,
                                Op::MenorIgual => x <= y,
                                _ => x >= y,
                            });
                            continue 'exec;
                        }
                        anotar!(c, a, b, Valor::int(0));
                        acc = Valor::bool(match op {
                            Op::Menor => t!(self.menor(a, b, true)) == Some(true),
                            Op::Maior => t!(self.menor(b, a, false)) == Some(true),
                            Op::MenorIgual => t!(self.menor(b, a, false)) == Some(false),
                            _ => t!(self.menor(a, b, true)) == Some(false),
                        });
                    }
                    Op::In => {
                        let r = u!();
                        if !self.e_objeto(acc) {
                            t!(self.erro_tipo::<()>("Cannot use 'in' operator to search for a key in a non-object"));
                        }
                        let k = t!(self.para_chave(reg!(r)));
                        acc = Valor::bool(t!(self.tem(acc, k)));
                    }
                    Op::Instanceof => {
                        let (r, _c) = (u!(), u!());
                        acc = Valor::bool(t!(self.instancia_de(reg!(r), acc)));
                    }
                    Op::SomaInt => {
                        let i = i4!();
                        let c = u!();
                        let a = acc;
                        acc = t!(self.somar_int(a, i));
                        if !(a.e_int() && acc.e_int()) {
                            anotar!(c, a, a, acc);
                        }
                    }
                    Op::Negativo => {
                        let c = u!();
                        let a = acc;
                        acc = t!(self.negativo(a));
                        anotar!(c, a, a, acc);
                    }
                    Op::ParaNumero => {
                        if !acc.e_int() {
                            acc = t!(self.para_numero_valor(acc));
                        }
                    }
                    Op::ParaNumerico => {
                        if !acc.e_int() {
                            acc = t!(self.para_numerico(acc));
                        }
                    }
                    Op::Nao => acc = Valor::bool(!self.verdadeiro(acc)),
                    Op::Til => {
                        let c = u!();
                        let a = acc;
                        acc = t!(self.til(a));
                        anotar!(c, a, a, acc);
                    }
                    Op::Typeof => acc = self.typeof_(acc),
                    Op::Incrementa | Op::Decrementa => {
                        let c = u!();
                        let a = acc;
                        acc = t!(self.mais_um(a, op == Op::Incrementa));
                        if !(a.e_int() && acc.e_int()) {
                            anotar!(c, a, a, acc);
                        }
                    }
                    Op::IncrementaReg | Op::DecrementaReg => {
                        let (r, c) = (u!(), u!());
                        let a = reg!(r);
                        if a.e_int() {
                            let n = a.como_int() + if op == Op::IncrementaReg { 1 } else { -1 };
                            if let Some(n) = Valor::int_se_cabe(i64::from(n)) {
                                reg!(r) = n;
                                // `i++` no fim do laço: o salto para trás vem logo depois.
                                if codigo.get(pc) == Some(&(Op::SaltaAtras as u8)) {
                                    pc = i32::from_le_bytes(codigo[pc + 1..pc + 5].try_into().unwrap()) as u32 as usize;
                                    volta!();
                                    osr!();
                                }
                                continue 'exec;
                            }
                        }
                        let v = if a.e_int() { a } else { t!(self.para_numerico(a)) };
                        let n = t!(self.mais_um(v, op == Op::IncrementaReg));
                        reg!(r) = n;
                        if !(a.e_int() && n.e_int()) {
                            anotar!(c, a, a, n);
                        }
                    }
                    Op::ParaInt32 => {
                        let c = u!();
                        if !acc.e_int() && self.e_numero(acc) {
                            let x = crate::numeros::para_int32(self.f64_de(acc));
                            acc = self.numero(f64::from(x));
                        } else if !acc.e_int() {
                            let a = acc;
                            acc = t!(self.bits(Bits::Ou, a, Valor::int(0)));
                            anotar!(c, a, a, acc);
                        }
                    }
                    Op::ParaTexto => acc = t!(self.para_texto(acc)),
                    Op::ParaChave => {
                        if !(acc.e_int() || self.e_texto(acc) || self.e_simbolo(acc)) {
                            let k = t!(self.para_chave(acc));
                            acc = self.chave_para_valor(k);
                        }
                    }
                    Op::ParaObjeto => acc = t!(self.para_objeto(acc)),

                    // ---- saltos ----
                    Op::Salta => pc = i4!() as u32 as usize,
                    Op::SaltaAtras => {
                        pc = i4!() as u32 as usize;
                        volta!();
                        osr!();
                    }
                    Op::SaltaSeVerdadeiro | Op::SaltaSeFalso => {
                        let a = i4!() as u32 as usize;
                        let v = if acc == Valor::TRUE { true } else if acc == Valor::FALSE { false } else { self.verdadeiro(acc) };
                        if v == (op == Op::SaltaSeVerdadeiro) {
                            pc = a;
                        }
                    }
                    Op::SaltaSeNuloOuIndef | Op::SaltaSeNaoNuloOuIndef => {
                        let a = i4!() as u32 as usize;
                        let n = acc.e_nulo_ou_indefinido() || acc == Valor::BURACO;
                        if n == (op == Op::SaltaSeNuloOuIndef) {
                            pc = a;
                        }
                    }
                    Op::SaltaSeIndef | Op::SaltaSeNaoIndef => {
                        let a = i4!() as u32 as usize;
                        if (acc == Valor::UNDEFINED) == (op == Op::SaltaSeIndef) {
                            pc = a;
                        }
                    }
                    Op::SaltaSeNaoBuraco => {
                        let a = i4!() as u32 as usize;
                        if acc != Valor::BURACO {
                            pc = a;
                        }
                    }

                    // ---- chamadas ----
                    Op::Chama | Op::ChamaEval => {
                        let (rf, rt, n) = (u!(), u!(), u!());
                        if op == Op::Chama {
                            let _c = u!();
                        }
                        let (f, this) = (reg!(rf), reg!(rt));
                        salvar_pc!();
                        let args = Args { base: (base as u32) + rt + 1, n };
                        if op == Op::ChamaEval && f == self.intr.eval {
                            let x = if n > 0 { reg!(rt + 1) } else { Valor::UNDEFINED };
                            acc = t!(self.eval_indireto(x));
                        } else {
                            depois_de_chamar!(self.entrar(f, this, args));
                        }
                    }
                    Op::ChamaEspalhado => {
                        let (rf, rt, _n) = (u!(), u!(), u!());
                        let (f, this, arr) = (reg!(rf), reg!(rt), reg!(rt + 1));
                        salvar_pc!();
                        let args = self.espalhar_na_pilha(arr);
                        depois_de_chamar!(self.entrar(f, this, args));
                    }
                    Op::New => {
                        let (rf, ra, n, _c) = (u!(), u!(), u!(), u!());
                        let f = reg!(rf);
                        salvar_pc!();
                        let args = Args { base: (base as u32) + ra, n };
                        depois_de_chamar!(self.construir_no_laco(f, args, f));
                    }
                    Op::NewEspalhado => {
                        let (rf, ra, _n) = (u!(), u!(), u!());
                        let (f, arr) = (reg!(rf), reg!(ra));
                        salvar_pc!();
                        let args = self.espalhar_na_pilha(arr);
                        depois_de_chamar!(self.construir_no_laco(f, args, f));
                    }
                    Op::SuperChama | Op::SuperChamaEspalhado => {
                        let (ra, n) = (u!(), u!());
                        salvar_pc!();
                        let args: Vec<Valor> = if op == Op::SuperChama {
                            (0..n).map(|i| reg!(ra + i)).collect()
                        } else {
                            t!(self.coletar_iteravel(reg!(ra)))
                        };
                        acc = t!(self.super_chamar(qi, &args, fb.flags.seta));
                    }
                    Op::Retorna => {
                        let q = self.quadros[qi];
                        let mut v = acc;
                        // Os erros do [[Construct]] vêm depois do corpo: o try/catch dele não os pega.
                        let mut erro = None;
                        if q.construindo && !self.e_objeto(v) {
                            if v != Valor::UNDEFINED && fb.flags.classe_derivada {
                                erro = self.erro_tipo::<()>("Derived constructors may only return object or undefined").err();
                            }
                            v = q.this;
                            if self.celula_this(v) {
                                v = self.lista_get(v, 0);
                            }
                            if v == Valor::BURACO && erro.is_none() {
                                erro = self.erro_referencia::<()>("Must call super constructor in derived class before accessing 'this' or returning from derived constructor").err();
                            }
                        }
                        self.quadros.pop();
                        if let Some(e) = erro {
                            if q.entrada {
                                self.pilha.truncate(q.topo as usize);
                                return Err(e);
                            }
                            let fim = self.quadros.last().unwrap().fim;
                            self.pilha.truncate(fim as usize);
                            pendente = Some((e, false));
                            continue 'quadro;
                        }
                        if q.entrada {
                            self.pilha.truncate(q.topo as usize);
                            return Ok(Fim::Retorno(v));
                        }
                        let fim = self.quadros.last().unwrap().fim;
                        self.pilha.truncate(fim as usize);
                        acc = v;
                        continue 'quadro;
                    }
                    Op::Lanca => {
                        self.quadros[qi].pc = ini as u32;
                        break 'exec acc;
                    }
                    Op::Relanca => {
                        let r = u!();
                        self.quadros[qi].pc = ini as u32;
                        break 'exec reg!(r);
                    }
                    Op::LancaErro => {
                        let k = u!();
                        let m = t!(self.constante(proto, k));
                        let e = self.erro_com(crate::intrinsecos::TipoErro::Tipo, m);
                        self.quadros[qi].pc = ini as u32;
                        break 'exec e;
                    }

                    // ---- iteração ----
                    Op::ObtemIterador => {
                        // n: bit 0 = assíncrono; bit 1 = iterador de verdade (sem o atalho de array).
                        let (r, n) = (u!(), u!());
                        let reg_it = if n & 1 != 0 {
                            t!(self.obter_iterador_async(acc))
                        } else if n & 2 != 0 {
                            t!(self.obter_iterador_real(acc))
                        } else {
                            t!(self.obter_iterador(acc))
                        };
                        reg!(r) = reg_it;
                        acc = Valor::UNDEFINED;
                    }
                    Op::ChamaProximo => {
                        let r = u!();
                        let reg = reg!(r);
                        salvar_pc!();
                        // Fica "acabado" até o resultado chegar: erro no next/await não fecha o iterador.
                        self.lista_set(reg, 2, Valor::TRUE);
                        let (it, next) = (self.lista_get(reg, 0), self.lista_get(reg, 1));
                        acc = t!(self.chamar(next, it, &[]));
                    }
                    Op::ResultadoProximo => {
                        let r = u!();
                        let a = i4!() as u32 as usize;
                        let reg = reg!(r);
                        salvar_pc!();
                        if !self.e_objeto(acc) {
                            t!(self.erro_tipo::<()>("Iterator result is not an object"));
                        }
                        let feito = t!(self.obter(acc, ChaveProp::Nome(fixos::DONE)));
                        if self.verdadeiro(feito) {
                            pc = a;
                        } else {
                            acc = t!(self.obter(acc, ChaveProp::Nome(fixos::VALUE)));
                            self.lista_set(reg, 2, Valor::FALSE);
                        }
                    }
                    Op::FechaIteradorAsync => {
                        let r = u!();
                        let a = i4!() as u32 as usize;
                        let reg = reg!(r);
                        salvar_pc!();
                        if self.lista_get(reg, 2) == Valor::TRUE {
                            pc = a;
                        } else {
                            self.lista_set(reg, 2, Valor::TRUE);
                            let it = self.lista_get(reg, 0);
                            match t!(self.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))) {
                                Some(f) => acc = t!(self.chamar(f, it, &[])),
                                None => pc = a,
                            }
                        }
                    }
                    Op::ExigeObjeto => {
                        if !self.e_objeto(acc) {
                            t!(self.erro_tipo::<()>("Iterator result is not an object"));
                        }
                    }
                    Op::Proximo => {
                        let r = u!();
                        let a = i4!() as u32 as usize;
                        let it = reg!(r);
                        salvar_pc!();
                        match t!(self.proximo(it)) {
                            Some(v) => acc = v,
                            None => pc = a,
                        }
                    }
                    Op::FechaIterador => {
                        let r = u!();
                        let it = reg!(r);
                        salvar_pc!();
                        t!(self.fechar_iterador(it));
                    }
                    Op::FechaIteradorEmErro => {
                        let r = u!();
                        let it = reg!(r);
                        salvar_pc!();
                        // IteratorClose com completude de exceção (§7.4.11 passo 5): o erro do
                        // `return` não substitui o que já estava sendo lançado. Fim do script não.
                        if let Err(e) = self.fechar_iterador(it) {
                            if self.terminando {
                                t!(Err::<(), _>(e));
                            }
                        }
                    }
                    Op::PreparaForIn => {
                        let r = u!();
                        let st = t!(self.preparar_for_in(acc));
                        reg!(r) = st;
                    }
                    Op::ProximoForIn => {
                        let r = u!();
                        let a = i4!() as u32 as usize;
                        let st = reg!(r);
                        match t!(self.proximo_for_in(st)) {
                            Some(k) => acc = k,
                            None => pc = a,
                        }
                    }

                    // ---- funções ----
                    Op::CriaArguments => {
                        let _ = u!();
                        let q = self.quadros[qi];
                        acc = self.criar_arguments(q.args, q.funcao);
                    }
                    Op::CriaResto => {
                        let n = u!();
                        let a = self.quadros[qi].args;
                        let itens: Vec<Valor> = (n..a.n.max(n)).map(|i| self.pilha[(a.base + i) as usize]).collect();
                        acc = self.array_de(&itens);
                    }
                    Op::Depurador => {}

                    // ---- geradores e async ----
                    Op::IniciaGerador => {
                        let assinc = fb.flags.assincrona;
                        let f = self.quadros[qi].funcao;
                        let p = t!(self.obter(f, ChaveProp::Nome(fixos::PROTOTYPE)));
                        let p = if self.e_objeto(p) {
                            p
                        } else if assinc {
                            self.intr.async_gerador_proto
                        } else {
                            self.intr.gerador_proto
                        };
                        let forma = self.forma_para(Classe::Gerador, p, 0);
                        let gen = self.novo_objeto(forma);
                        let id = self.novo_suspenso();
                        self.por_interno(gen, 0, id);
                        self.por_interno_valor(gen, 2, f);
                        if assinc {
                            // Gerador async: a fila de pedidos (e o estado) no campo 1.
                            use crate::builtins::assincronos::{AG_CAB, AG_INICIO};
                            let fila = self.nova_lista(AG_CAB);
                            self.lista_set(fila, 0, Valor::int(AG_INICIO));
                            self.lista_set(fila, 1, Valor::int(AG_CAB as i32));
                            self.por_interno_valor(gen, 1, fila);
                        }
                        let entrada = self.quadros[qi].entrada;
                        self.quadros[qi].suspenso = id;
                        let mut salvo = self.salvar_quadro(pc as u32);
                        if assinc {
                            salvo.promessa = gen;
                        }
                        self.suspensos[id as usize] = Some(salvo);
                        if entrada {
                            return Ok(Fim::Retorno(gen));
                        }
                        let fim = self.quadros.last().unwrap().fim;
                        self.pilha.truncate(fim as usize);
                        acc = gen;
                        continue 'quadro;
                    }
                    Op::Suspende | Op::Aguarda => {
                        let id = self.quadros[qi].suspenso;
                        assert!(self.quadros[qi].entrada && id != SEM_SUSPENSO, "yield/await fora de gerador ou async");
                        let mut salvo = self.salvar_quadro(pc as u32);
                        salvo.modo = if matches!(op, Op::Aguarda) { AGUARDA } else { RENDEU };
                        self.suspensos[id as usize] = Some(salvo);
                        return Ok(Fim::Suspenso(acc));
                    }
                    Op::CriaResultado => {
                        let n = u!();
                        acc = self.resultado_iterador(acc, n == 1);
                    }
                    Op::ModoRetomada => {
                        let a = i4!() as u32 as usize;
                        if std::mem::replace(&mut retomada, RETOMOU_NORMAL) == RETOMOU_RETORNA {
                            pc = a;
                        }
                    }
                    Op::Delega => {
                        let r = u!();
                        let a_fim = i4!() as u32 as usize;
                        let a_ret = i4!() as u32 as usize;
                        let reg = reg!(r);
                        let modo = std::mem::replace(&mut retomada, RETOMOU_NORMAL);
                        salvar_pc!();
                        match t!(self.delegar(reg, modo, acc, fb.flags.assincrona)) {
                            Passo::Fim(v) => {
                                acc = v;
                                pc = a_fim;
                            }
                            Passo::Retorno(v) => {
                                acc = v;
                                pc = a_ret;
                            }
                            Passo::Para(v, modo) => {
                                // Volta para o próprio Delega ao retomar.
                                let id = self.quadros[qi].suspenso;
                                let mut salvo = self.salvar_quadro(ini as u32);
                                salvo.modo = modo;
                                self.suspensos[id as usize] = Some(salvo);
                                return Ok(Fim::Suspenso(v));
                            }
                        }
                    }

                    Op::ImportDinamico => {
                        let r = u!();
                        acc = self.importar_dinamico(proto as u32, reg!(r));
                    }
                    Op::ImportMeta => acc = self.import_meta(proto as u32),
                    Op::LeImport => {
                        let k = u!();
                        acc = t!(self.ler_import(proto as u32, k));
                    }
                    Op::GuardaPadrao => self.guardar_padrao(proto as u32, acc),
                    Op::RegistraModulo => {
                        let c = self.quadros[qi].contexto;
                        self.registrar_contexto_modulo(proto as u32, c);
                    }
                    Op::CarregaNamespace => {
                        let n = u!();
                        acc = self.namespace_do_requisito(proto as u32, n);
                    }
                    Op::Fim => {
                        t!(self.erro_tipo::<()>("fim do código alcançado"));
                    }
                }
                continue 'exec;
            };
            pendente = Some((erro, true));
        }
    }

    // ---------------------------------------------------------------- auxiliares do laço

    fn constante(&mut self, proto: usize, k: u32) -> Res<Valor> {
        Ok(match self.protos[proto].consts[k as usize] {
            ConstRt::Valor(v) => v,
            ConstRt::Nome(c) => self.texto_do_nome(c),
            _ => Valor::UNDEFINED,
        })
    }

    /// SetFunctionName(f, chave, prefixo): n 0 sem prefixo, 1 `get `, 2 `set `. Símbolo vira
    /// `[descrição]`; nome privado (símbolo escondido), a própria descrição `#x`.
    fn nomear_funcao(&mut self, f: Valor, k: Valor, n: u32) -> Res<()> {
        if !self.e_funcao(f) || self.formas.get(self.forma(f)).classe != Classe::Funcao {
            return Ok(());
        }
        let mut u: Vec<u16> = ["", "get ", "set "][n as usize].encode_utf16().collect();
        if self.e_simbolo(k) {
            let d = self.descricao_do_simbolo(k);
            if self.escondidos.contains(&(self.chave_do_simbolo(k).0 & !Chave::BIT_SIMBOLO)) {
                u.extend(self.texto_u16_de(d));
            } else if d != Valor::UNDEFINED {
                u.push(b'[' as u16);
                u.extend(self.texto_u16_de(d));
                u.push(b']' as u16);
            }
        } else {
            let t = self.para_texto(k)?;
            u.extend(self.texto_u16_de(t));
        }
        let nome = self.texto_u16(&u);
        self.gravar_campo(f, 1, nome);
        Ok(())
    }

    // ---------------------------------------------------------------- elementos privados

    /// Os elementos privados moram no próprio objeto, sob o símbolo escondido do nome: a busca
    /// é só ali (sem protótipo, sem armadilha de Proxy). Campo = dado enumerável (assim
    /// `freeze`, que só tira gravável/configurável, não o confunde com método); método = dado
    /// não enumerável; getter/setter = par de acessores.
    fn privado(&self, o: Valor, nome: Valor) -> Option<crate::formas::Descritor> {
        if !self.e_objeto(o) {
            return None;
        }
        self.propria(o, self.chave_do_simbolo(nome))
    }

    fn sem_privado<T>(&mut self, nome: Valor) -> Res<T> {
        let d = self.descricao_do_simbolo(nome);
        let t = self.para_texto(d).map(|t| self.texto_rust(t)).unwrap_or_default();
        self.erro_tipo(&format!("Cannot access private member {t} on an object whose class did not declare it"))
    }

    /// PrivateGet.
    fn ler_privado(&mut self, o: Valor, nome: Valor) -> Res<Valor> {
        let Some(d) = self.privado(o, nome) else { return self.sem_privado(nome) };
        let v = self.ler_campo(o, d.slot);
        if d.attrs & attr::ACESSOR == 0 {
            return Ok(v);
        }
        let g = self.acessor_get(v);
        if g == Valor::UNDEFINED {
            return self.erro_tipo("'#' accessor was defined without a getter");
        }
        self.chamar(g, o, &[])
    }

    /// PrivateSet.
    fn gravar_privado(&mut self, o: Valor, nome: Valor, x: Valor) -> Res<()> {
        let Some(d) = self.privado(o, nome) else { return self.sem_privado(nome) };
        if d.attrs & attr::ACESSOR != 0 {
            let s = self.acessor_set(self.ler_campo(o, d.slot));
            if s == Valor::UNDEFINED {
                return self.erro_tipo("'#' accessor was defined without a setter");
            }
            return self.chamar(s, o, &[x]).map(|_| ());
        }
        if d.attrs & attr::ENUMERAVEL == 0 {
            return self.erro_tipo("Private method is not writable");
        }
        self.gravar_campo(o, d.slot, x);
        Ok(())
    }

    /// PrivateFieldAdd (`n` = 0) e PrivateMethodOrAccessorAdd (1 método, 2 getter, 3 setter).
    fn acrescentar_privado(&mut self, o: Valor, nome: Valor, v: Valor, n: u32) -> Res<()> {
        let c = self.chave_do_simbolo(nome);
        let atual = self.propria(o, c);
        if n >= 2 {
            // Getter e setter do mesmo nome chegam um depois do outro: o segundo completa o par.
            let par = atual.filter(|d| d.attrs & attr::ACESSOR != 0).map(|d| self.ler_campo(o, d.slot));
            let (mut g, mut s) = par.map_or((Valor::UNDEFINED, Valor::UNDEFINED), |p| (self.acessor_get(p), self.acessor_set(p)));
            if n == 2 { g = v } else { s = v }
            let par = self.novo_acessor(g, s);
            self.definir_propria(o, c, par, attr::ACESSOR);
            return Ok(());
        }
        if atual.is_some() {
            return self.erro_tipo("Cannot initialize private member twice on the same object");
        }
        let a = if n == 0 { attr::GRAVAVEL | attr::ENUMERAVEL } else { 0 };
        self.definir_propria(o, c, v, a);
        Ok(())
    }

    /// Protótipo da casa do método atual (`super`).
    fn super_base(&mut self, qi: usize) -> Res<Valor> {
        let f = self.quadros[qi].funcao;
        let casa = Valor(self.interno(f, F_CASA));
        if !self.e_objeto(casa) {
            return self.erro_tipo::<Valor>("'super' keyword unexpected here");
        }
        Ok(self.proto_de(casa))
    }

    /// Célula do `this` de construtor derivado com setas (ver `CriaFuncao`).
    #[inline]
    fn celula_this(&self, v: Valor) -> bool {
        v.e_ref() && self.forma(v) == crate::formas::LISTA
    }

    fn super_chamar(&mut self, qi: usize, args: &[Valor], seta: bool) -> Res<Valor> {
        let q = self.quadros[qi];
        // Em seta, o construtor, o new.target e o this vêm da célula dividida com ele.
        let c = if seta { Valor(self.interno(q.funcao, F_THIS)) } else { q.this };
        let celula = self.celula_this(c);
        let (f, nt0) = if celula { (self.lista_get(c, 1), self.lista_get(c, 2)) } else { (q.funcao, q.new_target) };
        let pai = self.proto_de(f);
        if !self.e_construtor(pai) {
            return self.erro_tipo("Super constructor is not a constructor");
        }
        let nt = if nt0 == Valor::UNDEFINED { f } else { nt0 };
        let o = self.construir(pai, args, nt)?;
        // BindThisValue depois de construir (a classe base roda mesmo no segundo `super()`).
        let ja = if celula { self.lista_get(c, 0) } else { self.quadros[qi].this };
        if ja != Valor::BURACO {
            return self.erro_referencia("Super constructor may only be called once");
        }
        if celula {
            self.lista_set(c, 0, o);
        } else {
            self.quadros[qi].this = o;
        }
        self.rodar_campos(f, o)?;
        Ok(o)
    }

    fn criar_classe(&mut self, proto: usize, k: u32, heranca: Valor, ctx: Valor) -> Res<Valor> {
        let ConstRt::Classe { construtor, derivada: _, nome } = self.protos[proto].consts[k as usize] else { panic!("CriaClasse sem classe") };
        let (proto_pai, ctor_pai) = if heranca == Valor::BURACO {
            (self.intr.objeto_proto, Valor::UNDEFINED)
        } else if heranca == Valor::NULL {
            (Valor::NULL, Valor::UNDEFINED)
        } else {
            if !self.e_construtor(heranca) {
                return self.erro_tipo("Class extends value is not a constructor or null");
            }
            let pp = self.obter(heranca, ChaveProp::Nome(fixos::PROTOTYPE))?;
            if !self.e_objeto(pp) && pp != Valor::NULL {
                return self.erro_tipo("Class extends value does not have valid prototype property");
            }
            (pp, heranca)
        };
        let fp = self.forma_para(Classe::Comum, proto_pai, 4);
        let p = self.novo_objeto(fp);
        let f = if construtor != u32::MAX {
            self.closure(construtor, ctx, Valor::UNDEFINED, Some(nome))
        } else {
            let id = if heranca == Valor::BURACO { self.intr.nativa_classe_base } else { self.intr.nativa_classe_derivada };
            let forma = self.intr.forma_funcao;
            let f = self.novo_objeto(forma);
            self.por_interno(f, F_CODIGO, id | BIT_NATIVA);
            self.gravar_campo(f, 0, Valor::int(0));
            self.gravar_campo(f, 1, nome);
            f
        };
        if ctor_pai != Valor::UNDEFINED {
            self.definir_proto(f, ctor_pai);
        }
        let d = self.propria(f, fixos::PROTOTYPE).expect("classe sem prototype");
        self.gravar_campo(f, d.slot, p);
        self.definir_propria(p, fixos::CONSTRUCTOR, f, attr::METODO);
        self.por_interno_valor(f, F_CASA, p);
        Ok(f)
    }

    fn registrar_campo(&mut self, classe: Valor, k: Valor, valor: Valor, tipo: u32) {
        let lista = match self.propria(classe, simb::CAMPOS) {
            Some(d) => self.ler_campo(classe, d.slot),
            None => Valor::LISTA_VAZIA,
        };
        let n = if lista == Valor::LISTA_VAZIA { 0 } else { self.lista_len(lista) };
        let nova = self.redimensionar_lista(lista, n + 3, Valor::UNDEFINED);
        self.lista_set(nova, n, k);
        self.lista_set(nova, n + 1, valor);
        self.lista_set(nova, n + 2, Valor::int(tipo as i32));
        self.definir_propria(classe, simb::CAMPOS, nova, 0);
    }

    fn objeto_template(&mut self, proto: usize, k: u32) -> Valor {
        if let ConstRt::Template(i) = self.protos[proto].consts[k as usize] {
            if i != u32::MAX {
                return self.templates[i as usize];
            }
        }
        let p = &self.protos[proto];
        let script = Rc::clone(&p.script);
        let Constante::Template { cozidos, crus } = p.codigo().constantes[k as usize].clone() else { return Valor::UNDEFINED };
        let mut vc = Vec::new();
        for c in &cozidos {
            vc.push(match c {
                Some(t) => self.texto_u16(&script.textos[*t as usize]),
                None => Valor::UNDEFINED,
            });
        }
        let mut vr = Vec::new();
        for t in &crus {
            vr.push(self.texto_u16(&script.textos[*t as usize]));
        }
        let arr = self.array_de(&vc);
        let raw = self.array_de(&vr);
        self.definir_propria(arr, fixos::RAW, raw, 0);
        self.congelar(raw);
        self.congelar(arr);
        let i = self.templates.len() as u32;
        self.templates.push(arr);
        self.protos[proto].consts[k as usize] = ConstRt::Template(i);
        arr
    }

    /// Fecha o objeto para extensões e torna as próprias só leitura.
    pub fn congelar(&mut self, o: Valor) {
        for d in self.chaves_proprias(o) {
            let a = if d.attrs & attr::ACESSOR != 0 { d.attrs & !attr::CONFIGURAVEL } else { d.attrs & !(attr::CONFIGURAVEL | attr::GRAVAVEL) };
            if a != d.attrs {
                let nf = self.formas.mudar_atributos(self.forma(o), d.chave, a);
                self.g.por_u32(o.deslocamento(), nf.0);
            }
        }
        let nf = self.formas.fechar(self.forma(o));
        self.g.por_u32(o.deslocamento(), nf.0);
    }

    /// Array novo com os itens.
    pub fn array_de(&mut self, itens: &[Valor]) -> Valor {
        let f = self.intr.forma_array;
        let a = self.novo_objeto(f);
        if itens.is_empty() {
            return a;
        }
        // A lista nasce do tamanho certo e o tipo dos elementos é decidido uma vez só (o mesmo que
        // `gravar_elemento` item a item daria: inteiros, ou valores).
        debug_assert!(!itens.contains(&Valor::BURACO));
        let n = itens.len() as u32;
        let el = self.nova_lista(n);
        for (i, &v) in itens.iter().enumerate() {
            self.lista_set(el, i as u32, v);
        }
        let d = a.deslocamento();
        self.g.por_valor(d, d + 8, el);
        if !itens.iter().all(|v| v.e_int()) {
            let nf = self.formas.com_elementos(f, crate::formas::TipoElementos::Valores);
            self.g.por_u32(d, nf.0);
        }
        self.por_interno(a, 0, n);
        a
    }

    fn criar_arguments(&mut self, args: Args, f: Valor) -> Valor {
        let forma = self.intr.forma_argumentos;
        let o = self.novo_objeto(forma);
        for i in 0..args.n {
            let v = self.pilha[(args.base + i) as usize];
            self.gravar_elemento(o, i, v);
        }
        self.definir_propria(o, fixos::LENGTH, Valor::int(args.n as i32), attr::METODO);
        self.definir_propria(o, fixos::CALLEE, f, attr::METODO);
        let it = self.intr.array_valores;
        self.definir_propria(o, simb::ITERATOR, it, attr::METODO);
        o
    }

    /// Copia as próprias enumeráveis de `src` para `dest`, menos as chaves `fora`.
    pub fn copiar_proprias(&mut self, dest: Valor, src: Valor, fora: &[ChaveProp]) -> Res<()> {
        for k in self.chaves_de(src, true, true, true) {
            if fora.contains(&k) {
                continue;
            }
            let v = self.obter(src, k)?;
            self.criar_dado(dest, k, v)?;
        }
        Ok(())
    }

    /// Espalha o iterável no topo da pilha, como argumentos.
    fn espalhar_na_pilha(&mut self, arr: Valor) -> Args {
        let base = self.pilha.len() as u32;
        let n = if self.e_array(arr) { self.array_len(arr) } else { 0 };
        for i in 0..n {
            let v = self.elemento(arr, i).unwrap_or(Valor::UNDEFINED);
            self.pilha.push(v);
        }
        Args { base, n }
    }

    fn eval_indireto(&mut self, x: Valor) -> Res<Valor> {
        if !self.e_texto(x) {
            return Ok(x);
        }
        let s = self.texto_rust(x);
        let p = self.compilar_texto(&s)?;
        let f = self.closure(p, Valor::UNDEFINED, Valor::UNDEFINED, None);
        let g = self.intr.global;
        self.chamar(f, g, &[])
    }

    /// Ponto seguro: aqui o coletor pode rodar (o acc está fora da pilha, então vai junto).
    /// Passou do [`crate::gaiola::TETO`]: coleta tudo (se pode) e diz se ainda não cabe.
    #[inline]
    fn sem_memoria(&mut self, acc: &mut Valor) -> bool {
        if self.g.cabe(0) || self.terminando {
            return false;
        }
        if self.pode_coletar() {
            let mut a = [*acc];
            self.coletar_tudo(&mut a);
            *acc = a[0];
        }
        !self.g.cabe(0)
    }

    /// N2: o código da função `proto`, compilando se ainda não tentou e `quente` (ou o modo
    /// pede tudo).
    fn jit_de(&mut self, proto: usize, quente: bool) -> Option<Rc<crate::jit::CodigoJit>> {
        let p = &self.protos[proto];
        if p.jit.is_some() || p.jit_tentou || !(quente || self.jit_modo == 2) {
            return p.jit.clone();
        }
        let c = crate::jit::compilar(self, proto).map(Rc::new);
        self.protos[proto].jit_tentou = true;
        self.jit_por(proto, c.clone());
        c
    }

    /// Troca o código do N2 da função `proto` (e a entrada dela na tabela da chamada direta).
    /// O código velho não é liberado: pode estar rodando mais abaixo na pilha de máquina.
    fn jit_por(&mut self, proto: usize, cod: Option<Rc<crate::jit::CodigoJit>>) {
        if let Some(velho) = self.protos[proto].jit.take() {
            self.jit_antigos.push(velho);
        }
        let p = &self.protos[proto];
        let fl = p.flags;
        let tam = p.registradores.max(p.parametros);
        let e = match &cod {
            Some(c) if !(fl.construtor_classe || fl.assincrona || fl.gerador) && c.regs <= tam => crate::jit::EntradaJit {
                endereco: c.endereco() as u64,
                tam,
                parametros: p.parametros,
                modo_this: if fl.seta { 0 } else if fl.estrita { 1 } else { 2 },
                _livre: 0,
            },
            _ => crate::jit::EntradaJit::default(),
        };
        if self.jit_tabela.len() <= proto {
            self.jit_tabela.resize(proto + 1, crate::jit::EntradaJit::default());
        }
        self.jit_tabela[proto] = e;
        self.protos[proto].jit = cod;
    }

    /// N2: a função chamada por uma nativa (callback de `map`, comparador de `sort`…) roda no
    /// código dela direto, sem passar pelo laço do intérprete. `Err` = como o laço deve continuar
    /// o quadro do topo (o código saiu para o N1, ou não há código).
    fn jit_externo(&mut self) -> Result<Valor, Entrada> {
        let topo = self.quadros.len() - 1;
        let proto = self.quadros[topo].proto as usize;
        if self.jit_modo == 0 || !self.jit_entrada(proto) {
            return Err(Entrada::Normal(Valor::UNDEFINED));
        }
        let Some(cod) = self.jit_de(proto, true) else { return Err(Entrada::Normal(Valor::UNDEFINED)) };
        let s = self.rodar_jit(cod, topo, 0, Valor::UNDEFINED);
        self.jit_externo_saida(topo, s)
    }

    /// O que [`Motor::jit_externo`] faz depois de rodar o código do quadro de entrada `topo`.
    fn jit_externo_saida(&mut self, topo: usize, s: SaidaJit) -> Result<Valor, Entrada> {
        match s {
            SaidaJit::Retorno(pc, v) => {
                let q = self.quadros[topo];
                if q.construindo {
                    self.quadros[topo].pc = pc as u32;
                    self.jit_pular = Some((topo as u32, pc as u32));
                    return Err(Entrada::Normal(v));
                }
                // O `Retorna` de um quadro de entrada.
                self.quadros.pop();
                self.pilha.truncate(q.topo as usize);
                Ok(v)
            }
            SaidaJit::Segue(pc, v) => {
                let t = self.quadros.len() - 1;
                self.quadros[t].pc = pc as u32;
                self.jit_pular = Some((t as u32, pc as u32));
                Err(Entrada::Normal(v))
            }
            SaidaJit::Aborto => match self.jit_erro.take() {
                // O laço lança do `pc` anterior ao guardado (o de depois da instrução).
                Some(e) => {
                    let t = self.quadros.len() - 1;
                    self.quadros[t].pc += 1;
                    Err(Entrada::Lanca(e))
                }
                None => Err(Entrada::Normal(self.jit_acc)),
            },
        }
    }

    /// N2: entra no código da função pelo começo ou na volta de uma chamada? Só se ele já existe
    /// ou a função ficou quente, e se o código não vive saindo para o N1.
    #[inline(always)]
    fn jit_entrada(&self, proto: usize) -> bool {
        let p = &self.protos[proto];
        (p.jit.is_some() || (!p.jit_tentou && (p.chamadas >= LIMITE_JIT || self.jit_modo == 2))) && p.jit_n1 < 1000
    }

    /// N2, no salto para trás que volta a `pc`: o código da função se ele entra ali. Laço quente
    /// que ficou de fora (o cache dele estava vazio quando a função compilou) recompila a função,
    /// poucas vezes.
    #[cold]
    #[inline(never)]
    fn jit_laco(&mut self, proto: usize, pc: usize) -> Option<Rc<crate::jit::CodigoJit>> {
        let quente = self.voltas & 0xFF == 0;
        let cod = self.jit_de(proto, quente)?;
        if cod.entra_em(pc) {
            return Some(cod);
        }
        let p = &mut self.protos[proto];
        if !quente || p.jit_recompilou >= 4 {
            return None;
        }
        p.jit_recompilou += 1;
        p.jit_tentou = false;
        self.jit_por(proto, None);
        self.jit_de(proto, true).filter(|c| c.entra_em(pc))
    }

    /// N2: roda o código `cod` do quadro `qi` a partir de `pc` (uma entrada). Entre lotes de
    /// voltas faz o que o `volta!` faz: calor, interrupção, coleta, memória.
    fn rodar_jit(&mut self, mut cod: Rc<crate::jit::CodigoJit>, qi: usize, mut pc: usize, mut acc: Valor) -> SaidaJit {
        let (proto, base) = (self.quadros[qi].proto as usize, self.quadros[qi].base as usize);
        // O código lê e grava os registradores direto: todos têm de existir. E a pilha de
        // máquina: cada código aninhado é um quadro nativo a mais.
        if base + cod.regs as usize > self.pilha.len() || self.jit_profundidade >= 200 {
            return SaidaJit::Segue(pc, acc);
        }
        if self.jit_profundidade == 0 {
            // Até 256 KB de pilha de máquina para os códigos chamados direto, a partir daqui.
            self.jit_limite = endereco_pilha().saturating_sub(256 * 1024);
        }
        self.jit_profundidade += 1;
        let (topo, teto) = self.g.ptr_topo_teto();
        let r = loop {
            let m: *mut Motor = self;
            let mut ctx = crate::jit::Contexto {
                regs: self.pilha[base..].as_mut_ptr(),
                gaiola: self.g.ptr(),
                pendente: std::ptr::addr_of!(self.g.coleta_pendente),
                motor: m,
                acc: acc.0,
                pc: pc as u32,
                voltas: 0,
                motivo: 0,
                formas: self.formas.ptr(),
                n_formas: self.formas.len() as u32,
                classes: self.formas.ptr_tc(),
                limite: self.jit_limite,
                caixa: 0,
                quadro: qi as u32,
                base: base as u32,
                contexto: self.quadros[qi].contexto.0,
                topo,
                teto,
            };
            // SEGURANÇA: `regs` tem `cod.regs` registradores (conferido acima; as nativas que o
            // código chama renovam os ponteiros), `pc` é uma entrada, e o motor só é usado pelas
            // nativas do próprio código enquanto ele roda.
            unsafe { cod.rodar(&mut ctx) };
            acc = Valor(ctx.acc);
            pc = ctx.pc as usize;
            if let Some(s) = self.jit_voltas(proto, qi, pc, ctx.voltas) {
                break s;
            }
            match ctx.motivo {
                crate::jit::SAIU_LOTE => {
                    if self.g.coleta_pendente && self.pode_coletar() {
                        self.quadros[qi].pc = pc as u32;
                        acc = self.ponto_seguro(acc);
                    }
                    if self.sem_memoria(&mut acc) {
                        let e = self.sem_memoria_erro().unwrap_err();
                        break self.jit_erro_em(qi, pc, e);
                    }
                    if !cod.entra_em(pc) {
                        break SaidaJit::Segue(pc, acc);
                    }
                    // Cache de propriedade que estava vazio ao compilar já aprendeu: recompila
                    // (poucas vezes) para o caminho rápido dele entrar no código.
                    if cod.frio(self) && self.protos[proto].jit_recompilou < 4 {
                        let p = &mut self.protos[proto];
                        p.jit_recompilou += 1;
                        p.jit_tentou = false;
                        self.jit_por(proto, None);
                        match self.jit_de(proto, true) {
                            Some(novo) if novo.entra_em(pc) => cod = novo,
                            _ => break SaidaJit::Segue(pc, acc),
                        }
                    }
                }
                crate::jit::SAIU_GUARDA => {
                    let p = &mut self.protos[proto];
                    p.jit_falhas += 1;
                    if p.jit_falhas >= crate::jit::MAX_FALHAS {
                        self.jit_por(proto, None);
                    }
                    break SaidaJit::Segue(pc, acc);
                }
                crate::jit::SAIU_RETORNO => break SaidaJit::Retorno(pc, acc),
                crate::jit::SAIU_ABORTO => break SaidaJit::Aborto,
                _ => {
                    let p = &mut self.protos[proto];
                    p.jit_n1 = p.jit_n1.saturating_add(1);
                    break SaidaJit::Segue(pc, acc);
                }
            }
        };
        self.jit_profundidade -= 1;
        r
    }

    /// As voltas que o código fez (o que o `volta!` conta: calor e o `interromper`). Some = o
    /// script foi interrompido.
    fn jit_voltas(&mut self, proto: usize, qi: usize, pc: usize, n: u32) -> Option<SaidaJit> {
        if n == 0 {
            return None;
        }
        let antes = self.voltas;
        self.voltas = antes.wrapping_add(n);
        let amostras = (self.voltas >> 8).wrapping_sub(antes >> 8);
        let p = &mut self.protos[proto];
        p.voltas = p.voltas.saturating_add(amostras.saturating_mul(256));
        if (self.voltas >> 16) != (antes >> 16) {
            if let Some(motivo) = self.interromper.as_mut().and_then(|f| f()) {
                let e = self.erro(crate::intrinsecos::TipoErro::Erro, &motivo);
                return Some(self.jit_erro_em(qi, pc, e));
            }
        }
        None
    }

    /// O código chamado direto saiu de outro jeito (lote de voltas, guarda, instrução que o N2
    /// não sabe, erro): o mesmo que [`Motor::jit_chamar`] faz depois de rodar o código.
    pub(crate) fn jit_continuar_direto(&mut self, qi: usize, topo: usize, motivo: u32, voltas: u32, pc: usize, acc: Valor) -> Option<Valor> {
        let saida = self.jit_motivo(topo, motivo, voltas, pc, acc);
        self.jit_fim_chamada(qi, topo, saida)
    }

    /// O código do quadro `topo` saiu com `motivo` (o que `rodar_jit` faz depois de cada lote).
    fn jit_motivo(&mut self, topo: usize, motivo: u32, voltas: u32, pc: usize, acc: Valor) -> SaidaJit {
        let proto = self.quadros[topo].proto as usize;
        match self.jit_voltas(proto, topo, pc, voltas) {
            Some(s) => s,
            None => match motivo {
                crate::jit::SAIU_LOTE => match self.protos[proto].jit.clone() {
                    Some(cod) if cod.entra_em(pc) => self.rodar_jit(cod, topo, pc, acc),
                    _ => SaidaJit::Segue(pc, acc),
                },
                crate::jit::SAIU_GUARDA => {
                    let p = &mut self.protos[proto];
                    p.jit_falhas += 1;
                    if p.jit_falhas >= crate::jit::MAX_FALHAS {
                        self.jit_por(proto, None);
                    }
                    SaidaJit::Segue(pc, acc)
                }
                crate::jit::SAIU_RETORNO => SaidaJit::Retorno(pc, acc),
                crate::jit::SAIU_ABORTO => SaidaJit::Aborto,
                _ => {
                    let p = &mut self.protos[proto];
                    p.jit_n1 = p.jit_n1.saturating_add(1);
                    SaidaJit::Segue(pc, acc)
                }
            },
        }
    }

    /// Depois do código da função chamada (quadro `topo`) rodar: o valor de volta, ou None =
    /// desistir (o intérprete continua do quadro do topo).
    fn jit_fim_chamada(&mut self, qi: usize, topo: usize, saida: SaidaJit) -> Option<Valor> {
        match saida {
            SaidaJit::Retorno(pc, v) => {
                let q = &self.quadros[topo];
                if q.construindo || q.entrada {
                    self.jit_abortar_em(topo, pc, v);
                    return None;
                }
                // O `Retorna` do intérprete, sem construtor nem entrada.
                self.quadros.pop();
                let fim = self.quadros[qi].fim;
                self.pilha.truncate(fim as usize);
                Some(v)
            }
            SaidaJit::Segue(pc, v) => {
                self.jit_abortar_em(topo, pc, v);
                None
            }
            SaidaJit::Aborto => None,
        }
    }

    /// Erro no quadro `qi` em `pc`: o intérprete lança dali (pela saída [`SaidaJit::Aborto`]).
    fn jit_erro_em(&mut self, qi: usize, pc: usize, e: Valor) -> SaidaJit {
        self.quadros[qi].pc = pc as u32;
        self.reanotar_erro(e, qi, pc);
        self.jit_erro = Some(e);
        SaidaJit::Aborto
    }

    /// O código do quadro `qi` aninhado saiu para o N1 em `pc`: os de baixo desistem e o
    /// intérprete continua deste, sem voltar ao código antes de rodar a instrução.
    fn jit_abortar_em(&mut self, qi: usize, pc: usize, acc: Valor) {
        self.quadros[qi].pc = pc as u32;
        self.jit_acc = acc;
        self.jit_pular = Some((qi as u32, pc as u32));
    }

    /// `Chama rf, rt, n` do código do quadro `qi`. Função de bytecode com código roda aninhada;
    /// None = desistir (o intérprete continua do quadro do topo; erro em `jit_erro`).
    pub(crate) fn jit_chamar(&mut self, qi: usize, rf: u32, rt: u32, n: u32, ini: usize, prox: usize) -> Option<Valor> {
        // O mesmo que o intérprete faz antes de chamar (`salvar_pc!`); e o ponto seguro dele nas
        // chamadas: o acumulador do chamador não está vivo, os registradores estão na pilha.
        self.quadros[qi].pc = prox as u32;
        if self.g.coleta_pendente && self.pode_coletar() {
            self.ponto_seguro(Valor::UNDEFINED);
        }
        let base = self.quadros[qi].base;
        let (f, this) = (self.pilha[(base + rf) as usize], self.pilha[(base + rt) as usize]);
        let args = Args { base: base + rt + 1, n };
        let cod = match self.jit_empurrar(f, this, args) {
            // Função com código: o quadro já está empurrado.
            Some(p) => self.protos[p as usize].jit.clone(),
            None => {
                let r = match self.entrar(f, this, args) {
                    Ok(r) => r,
                    Err(e) => {
                        let fim = self.quadros[qi].fim;
                        self.pilha.truncate(fim as usize);
                        self.jit_erro_em(qi, ini, e);
                        return None;
                    }
                };
                if r.is_some() {
                    return r;
                }
                let proto = self.quadros[self.quadros.len() - 1].proto as usize;
                if self.jit_entrada(proto) {
                    self.jit_de(proto, true)
                } else {
                    None
                }
            }
        };
        let topo = self.quadros.len() - 1;
        let Some(cod) = cod else {
            // Sem código: o intérprete roda a função chamada desde o começo.
            self.jit_acc = Valor::UNDEFINED;
            self.jit_pular = None;
            return None;
        };
        let saida = self.rodar_jit(cod, topo, 0, Valor::UNDEFINED);
        self.jit_fim_chamada(qi, topo, saida)
    }

    /// [`Motor::anotar`] do código do quadro `qi`: soma `a + b` que saiu dos inteiros.
    pub(crate) fn jit_anotar(&mut self, qi: usize, c: u32, a: Valor, b: Valor) {
        let proto = self.quadros[qi].proto as usize;
        if self.protos[proto].tipos[c as usize] < tipo::ESTAVEL {
            // O resultado não é inteiro (por isso o caminho lento): `estourou` se eram dois.
            self.anotar(proto, c, a, b, Valor::UNDEFINED);
        }
    }

    /// O caminho geral de uma instrução do código do quadro `qi` (o caminho rápido dela, quando
    /// há, já ficou no código): o mesmo que o intérprete faz, com o acumulador `acc` e os
    /// operandos `a`, `b`, `c`. Devolve o novo acumulador; None = erro (em `jit_erro`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn jit_instrucao(&mut self, qi: usize, op: Op, a: u32, b: u32, c: u32, ini: usize, acc: Valor) -> Option<Valor> {
        let proto = self.quadros[qi].proto as usize;
        let base = self.quadros[qi].base as usize;
        let reg = |m: &Motor, r: u32| m.pilha[base + r as usize];
        let nome = |m: &Motor, k: u32| match m.protos[proto].consts[k as usize] {
            ConstRt::Nome(c) => c,
            _ => panic!("constante não é nome"),
        };
        let estrito = |m: &Motor| m.protos[proto].codigo().flags.estrita;
        let r: Res<Valor> = (|| match op {
            Op::LeGlobal => {
                let c = nome(self, a);
                self.ler_global_cache(proto, b, c, false)
            }
            Op::GravaGlobal => {
                let c = nome(self, a);
                self.gravar_global_cache(proto, b, c, acc, estrito(self)).map(|_| acc)
            }
            Op::LeContexto | Op::LeContextoChecado => {
                let ctx = self.ctx_subir(self.quadros[qi].contexto, a);
                let v = self.ctx_ler(ctx, b);
                if v == Valor::BURACO && op == Op::LeContextoChecado {
                    self.erro_referencia("Cannot access variable before initialization")
                } else {
                    Ok(v)
                }
            }
            Op::GravaContexto => {
                let ctx = self.ctx_subir(self.quadros[qi].contexto, a);
                self.ctx_gravar(ctx, b, acc);
                Ok(acc)
            }
            Op::Soma => {
                let x = reg(self, a);
                let v = self.somar(x, acc)?;
                self.jit_perfil(proto, b, x, acc, v);
                Ok(v)
            }
            Op::Sub | Op::Mul | Op::Div | Op::Resto | Op::Potencia => {
                let ar = match op {
                    Op::Sub => Arit::Sub,
                    Op::Mul => Arit::Mul,
                    Op::Div => Arit::Div,
                    Op::Resto => Arit::Resto,
                    _ => Arit::Potencia,
                };
                let x = reg(self, a);
                let v = self.aritmetica(ar, x, acc)?;
                self.jit_perfil(proto, b, x, acc, v);
                Ok(v)
            }
            Op::E | Op::Ou | Op::OuExclusivo | Op::DeslEsq | Op::DeslDir | Op::DeslDirSemSinal => {
                let bt = match op {
                    Op::E => Bits::E,
                    Op::Ou => Bits::Ou,
                    Op::OuExclusivo => Bits::Xor,
                    Op::DeslEsq => Bits::Esq,
                    Op::DeslDir => Bits::Dir,
                    _ => Bits::DirSemSinal,
                };
                let x = reg(self, a);
                let v = self.bits(bt, x, acc)?;
                self.jit_perfil(proto, b, x, acc, v);
                Ok(v)
            }
            Op::SomaInt => {
                let v = self.somar_int(acc, a as i32)?;
                if !(acc.e_int() && v.e_int()) {
                    self.jit_perfil(proto, b, acc, acc, v);
                }
                Ok(v)
            }
            Op::Igual | Op::Diferente => {
                let x = reg(self, a);
                self.jit_perfil(proto, b, x, acc, Valor::int(0));
                let i = self.iguais(x, acc)?;
                Ok(Valor::bool(i == (op == Op::Igual)))
            }
            Op::Identico | Op::NaoIdentico => {
                let x = reg(self, a);
                let i = self.identicos(x, acc);
                Ok(Valor::bool(i == (op == Op::Identico)))
            }
            Op::Menor | Op::Maior | Op::MenorIgual | Op::MaiorIgual => {
                let x = reg(self, a);
                self.jit_perfil(proto, b, x, acc, Valor::int(0));
                Ok(Valor::bool(match op {
                    Op::Menor => self.menor(x, acc, true)? == Some(true),
                    Op::Maior => self.menor(acc, x, false)? == Some(true),
                    Op::MenorIgual => self.menor(acc, x, false)? == Some(false),
                    _ => self.menor(x, acc, true)? == Some(false),
                }))
            }
            // Os saltos condicionais: o booleano do acumulador (que fica como está).
            Op::SaltaSeFalso | Op::SaltaSeVerdadeiro | Op::Nao => {
                let v = self.verdadeiro(acc);
                Ok(Valor::bool(if op == Op::Nao { !v } else { v }))
            }
            Op::Incrementa | Op::Decrementa => {
                let v = self.mais_um(acc, op == Op::Incrementa)?;
                if !(acc.e_int() && v.e_int()) {
                    self.jit_perfil(proto, a, acc, acc, v);
                }
                Ok(v)
            }
            Op::IncrementaReg | Op::DecrementaReg => {
                let x = reg(self, a);
                let v = if x.e_int() { x } else { self.para_numerico(x)? };
                let n = self.mais_um(v, op == Op::IncrementaReg)?;
                self.pilha[base + a as usize] = n;
                if !(x.e_int() && n.e_int()) {
                    self.jit_perfil(proto, b, x, x, n);
                }
                Ok(acc)
            }
            Op::ParaNumerico => {
                if acc.e_int() {
                    Ok(acc)
                } else {
                    self.para_numerico(acc)
                }
            }
            Op::ParaInt32 => {
                if acc.e_int() {
                    Ok(acc)
                } else if self.e_numero(acc) {
                    let x = crate::numeros::para_int32(self.f64_de(acc));
                    Ok(self.numero(f64::from(x)))
                } else {
                    let v = self.bits(Bits::Ou, acc, Valor::int(0))?;
                    self.jit_perfil(proto, a, acc, acc, v);
                    Ok(v)
                }
            }
            Op::LeProp => {
                let o = reg(self, a);
                let c_ = nome(self, b);
                self.ler_prop_cache(proto, c, o, c_)
            }
            Op::LePropAcc => {
                let c_ = nome(self, a);
                self.ler_prop_cache(proto, b, acc, c_)
            }
            Op::GravaProp => {
                let o = reg(self, a);
                let c_ = nome(self, b);
                let e = estrito(self);
                self.gravar_prop_cache(proto, c, o, c_, acc, e).map(|_| acc)
            }
            Op::LeItem => {
                let o = reg(self, a);
                self.op_le_item(o, acc)
            }
            Op::GravaItem => {
                let (o, kv) = (reg(self, a), reg(self, b));
                let e = estrito(self);
                self.op_grava_item(o, kv, acc, e).map(|_| acc)
            }
            Op::CriaObjeto => Ok(self.op_cria_objeto(a)),
            Op::CriaArray => {
                let f = self.intr.forma_array;
                Ok(self.novo_objeto(f))
            }
            Op::DefProp => {
                let o = reg(self, a);
                let c_ = nome(self, b);
                self.op_def_prop(proto, c, o, c_, acc);
                Ok(acc)
            }
            Op::Empurra => {
                let arr = reg(self, a);
                self.op_empurra(arr, acc);
                Ok(acc)
            }
            Op::CriaFuncao => Ok(self.op_cria_funcao(qi, proto, a)),
            Op::Typeof => Ok(self.typeof_(acc)),
            Op::CarregaConst => self.constante(proto, a),
            _ => unreachable!("o N2 não pede {op:?}"),
        })();
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.jit_erro_em(qi, ini, e);
                None
            }
        }
    }

    /// O `anotar!` do intérprete, para o caminho geral do N2.
    #[inline]
    fn jit_perfil(&mut self, proto: usize, c: u32, a: Valor, b: Valor, r: Valor) {
        if self.protos[proto].tipos.get(c as usize).is_some_and(|&t| t < tipo::ESTAVEL) {
            self.anotar(proto, c, a, b, r);
        }
    }

    /// `CriaObjeto n`.
    #[inline(always)]
    fn op_cria_objeto(&mut self, n: u32) -> Valor {
        let f = if n <= 4 {
            self.intr.forma_objeto
        } else {
            let op = self.intr.objeto_proto;
            self.forma_para(Classe::Comum, op, n.min(64))
        };
        self.novo_objeto(f)
    }

    /// `DefProp`: cache de transição do objeto literal: forma de antes (`forma`), forma depois
    /// (`forma_dono`) e o slot do campo novo.
    #[inline(always)]
    fn op_def_prop(&mut self, proto: usize, ci: u32, o: Valor, c: Chave, v: Valor) {
        let antes = self.forma(o).0;
        let e = self.protos[proto].caches[ci as usize];
        if e.forma == antes {
            self.g.por_u32(o.deslocamento(), e.forma_dono);
            self.gravar_campo(o, e.deslocamento, v);
        } else {
            self.definir_propria(o, c, v, attr::PADRAO);
            let depois = self.forma(o);
            if depois.0 != antes && !self.formas.get(depois).dicionario {
                if let Some(d) = self.formas.procurar(depois, c) {
                    let e = &mut self.protos[proto].caches[ci as usize];
                    (e.forma, e.forma_dono, e.deslocamento) = (antes, depois.0, d.slot);
                }
            }
        }
    }

    /// `Empurra`: o item no fim do array literal.
    #[inline(always)]
    fn op_empurra(&mut self, a: Valor, v: Valor) {
        let n = self.array_len(a);
        self.gravar_elemento(a, n, v);
    }

    /// `o[k]`.
    #[inline(always)]
    fn op_le_item(&mut self, o: Valor, k: Valor) -> Res<Valor> {
        if k.e_int() && k.como_int() >= 0 && self.e_objeto(o) {
            if let Some(v) = self.elemento(o, k.como_int() as u32) {
                return Ok(v);
            }
            if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado {
                return Ok(self.tipado_ler(o, k.como_int() as u32).unwrap_or(Valor::UNDEFINED));
            }
        }
        let k = self.para_chave(k)?;
        self.obter(o, k)
    }

    /// `o[kv] = v`.
    #[inline(always)]
    fn op_grava_item(&mut self, o: Valor, kv: Valor, v: Valor, estrito: bool) -> Res<()> {
        if kv.e_int() && kv.como_int() >= 0 && self.e_objeto(o) {
            let i = kv.como_int() as u32;
            let classe = self.formas.get(self.forma(o)).classe;
            if classe == Classe::ArrayTipado {
                return self.tipado_gravar(o, i, v);
            }
            // Item que já está na lista é comum (os especiais saem dela); item novo só sem índices especiais.
            let existe = self.elemento(o, i).is_some();
            if matches!(classe, Classe::Array | Classe::Comum | Classe::Argumentos) && (existe || (!self.indices_especiais && (self.proto_de(o) == self.intr.array_proto || self.proto_de(o) == self.intr.objeto_proto))) {
                if self.gravar_elemento(o, i, v) {
                    return Ok(());
                }
            }
        }
        let k = self.para_chave(kv)?;
        self.gravar(o, k, v, estrito)
    }

    /// `CriaFuncao k` no quadro `qi`.
    fn op_cria_funcao(&mut self, qi: usize, proto: usize, k: u32) -> Valor {
        let ConstRt::Funcao(p) = self.protos[proto].consts[k as usize] else { panic!("CriaFuncao sem função") };
        let fl = self.protos[proto].codigo().flags;
        let q = self.quadros[qi];
        let mut this = if fl.seta { Valor(self.interno(q.funcao, F_THIS)) } else { q.this };
        if fl.construtor_classe && fl.classe_derivada && !self.celula_this(this) && self.protos[p as usize].codigo().flags.seta {
            // Seta no construtor derivado: o `this` (e o `super()`) fica numa célula
            // [this, construtor, new.target] dividida com o construtor.
            let c = self.nova_lista(3);
            self.lista_set(c, 0, this);
            self.lista_set(c, 1, q.funcao);
            self.lista_set(c, 2, q.new_target);
            self.quadros[qi].this = c;
            this = c;
        }
        let f = self.closure(p, q.contexto, this, None);
        if fl.seta || self.protos[p as usize].codigo().flags.seta {
            // A seta herda a casa (super) de quem a cria.
            let casa = Valor(self.interno(q.funcao, F_CASA));
            if q.funcao != Valor::UNDEFINED && casa != Valor::UNDEFINED {
                self.por_interno_valor(f, F_CASA, casa);
            }
        }
        f
    }

    /// `LeGlobal`: o campo do objeto global pelo cache `ci`, ou o caminho geral (que enche o
    /// cache).
    #[inline(always)]
    fn ler_global_cache(&mut self, proto: usize, ci: u32, c: Chave, typeof_: bool) -> Res<Valor> {
        let g = self.intr.global;
        let cache = self.protos[proto].caches[ci as usize];
        if cache.forma == self.forma(g).0 && cache.forma_dono == self.epoca_lexico {
            return Ok(self.ler_campo(g, cache.deslocamento));
        }
        let v = self.ler_global(c, typeof_)?;
        if !self.lexico.contains_key(&c.0) && !self.formas.get(self.forma(g)).dicionario {
            if let Some(d) = self.propria(g, c) {
                if d.attrs & attr::ACESSOR == 0 {
                    let fg = self.forma(g).0;
                    let ep = self.epoca_lexico;
                    let cp = &mut self.protos[proto].caches[ci as usize];
                    cp.forma = fg;
                    cp.deslocamento = d.slot;
                    cp.forma_dono = ep;
                }
            }
        }
        Ok(v)
    }

    /// `GravaGlobal`, como [`Motor::ler_global_cache`].
    #[inline(always)]
    fn gravar_global_cache(&mut self, proto: usize, ci: u32, c: Chave, v: Valor, estrito: bool) -> Res<()> {
        let g = self.intr.global;
        let cache = self.protos[proto].caches[ci as usize];
        if cache.forma == self.forma(g).0 && cache.forma_dono == self.epoca_lexico {
            self.gravar_campo(g, cache.deslocamento, v);
            return Ok(());
        }
        self.gravar_global(c, v, estrito)?;
        if !self.lexico.contains_key(&c.0) && !self.formas.get(self.forma(g)).dicionario {
            if let Some(d) = self.propria(g, c) {
                if d.attrs & (attr::ACESSOR | attr::GRAVAVEL) == attr::GRAVAVEL {
                    let fg = self.forma(g).0;
                    let ep = self.epoca_lexico;
                    let cp = &mut self.protos[proto].caches[ci as usize];
                    cp.forma = fg;
                    cp.deslocamento = d.slot;
                    cp.forma_dono = ep;
                }
            }
        }
        Ok(())
    }

    fn sem_memoria_erro(&mut self) -> Res<Valor> {
        self.terminando = true;
        self.erro_alcance("Out of memory")
    }

    /// O bit de [`tipo`] de um valor.
    #[inline]
    fn perfil(&self, v: Valor) -> u8 {
        use crate::formas::{BIGINT, CORDA, LATIN1, NUMERO, UTF16};
        if v.e_int() {
            return tipo::INT;
        }
        match self.forma(v) {
            NUMERO => tipo::DOUBLE,
            LATIN1 | UTF16 | CORDA => tipo::TEXTO,
            BIGINT => tipo::BIGINT,
            _ => tipo::OUTRO,
        }
    }

    /// Anota no cache `c` os operandos `a` e `b` e se a conta saiu dos inteiros (`r`). Fora de
    /// linha: dentro do `laco` gigante ela piorava os registradores do caminho rápido (+12%).
    #[cold]
    #[inline(never)]
    fn anotar(&mut self, proto: usize, c: u32, a: Valor, b: Valor, r: Valor) {
        let mut t = self.perfil(a) | self.perfil(b);
        if a.e_int() && b.e_int() && !r.e_int() {
            t |= tipo::ESTOUROU;
        }
        let e = &mut self.protos[proto].tipos[c as usize];
        // Os 2 bits de cima contam anotações seguidas sem novidade; na 3ª o cache fica estável.
        *e = if *e & t == t { *e + tipo::UMA_SEM_NOVIDADE } else { (*e | t) & !tipo::ESTAVEL };
    }

    fn ponto_seguro(&mut self, acc: Valor) -> Valor {
        let mut a = [acc];
        self.coletar(&mut a);
        a[0]
    }

    // ---------------------------------------------------------------- iteradores

    /// `{value, done}`.
    pub fn resultado_iterador(&mut self, v: Valor, feito: bool) -> Valor {
        let o = self.objeto_vazio();
        self.definir_propria(o, fixos::VALUE, v, attr::PADRAO);
        self.definir_propria(o, fixos::DONE, Valor::bool(feito), attr::PADRAO);
        o
    }

    /// Registro de iterador: lista `[iterador, next, feito, fase do yield*]`; array comum:
    /// `[array, IT_ARRAY, i]`.
    pub fn obter_iterador(&mut self, v: Valor) -> Res<Valor> {
        if self.e_array(v) && self.iterador_de_array_intacto(v) {
            let reg = self.nova_lista(3);
            self.lista_set(reg, 0, v);
            self.lista_set(reg, 1, IT_ARRAY);
            self.lista_set(reg, 2, Valor::int(0));
            return Ok(reg);
        }
        self.obter_iterador_real(v)
    }

    /// GetIterator(v, sync) sem o atalho de array (o `yield*` fala com o iterador de verdade).
    pub fn obter_iterador_real(&mut self, v: Valor) -> Res<Valor> {
        let reg = self.nova_lista(4);
        let f = self.obter(v, ChaveProp::Nome(simb::ITERATOR))?;
        if !self.e_funcao(f) {
            let d = self.descrever(v);
            return self.erro_tipo(&format!("{d} is not iterable"));
        }
        let it = self.chamar(f, v, &[])?;
        if !self.e_objeto(it) {
            return self.erro_tipo("Result of the Symbol.iterator method is not an object");
        }
        let next = self.obter(it, ChaveProp::Nome(fixos::NEXT))?;
        self.lista_set(reg, 0, it);
        self.lista_set(reg, 1, next);
        self.lista_set(reg, 2, Valor::FALSE);
        self.lista_set(reg, 3, Valor::int(0));
        Ok(reg)
    }

    /// GetIterator(v, async): `[Symbol.asyncIterator]`, ou o iterador comum embrulhado
    /// (CreateAsyncFromSyncIterator).
    pub fn obter_iterador_async(&mut self, v: Valor) -> Res<Valor> {
        let (it, next) = match self.obter_metodo(v, ChaveProp::Nome(simb::ASYNC_ITERATOR))? {
            Some(f) => {
                let it = self.chamar(f, v, &[])?;
                if !self.e_objeto(it) {
                    return self.erro_tipo("Result of the Symbol.asyncIterator method is not an object");
                }
                let next = self.obter(it, ChaveProp::Nome(fixos::NEXT))?;
                (it, next)
            }
            None => {
                let sync = self.obter_iterador_real(v)?;
                let w = self.async_de_sync(sync);
                let next = self.obter(w, ChaveProp::Nome(fixos::NEXT))?;
                (w, next)
            }
        };
        let reg = self.nova_lista(4);
        self.lista_set(reg, 0, it);
        self.lista_set(reg, 1, next);
        self.lista_set(reg, 2, Valor::FALSE);
        self.lista_set(reg, 3, Valor::int(0));
        Ok(reg)
    }

    /// Um passo do `yield*` sobre o registro `reg` (§15.5.5, YieldExpression: yield* ...),
    /// retomado com `modo` e o valor `v`. Em gerador async cada resultado do de dentro é
    /// aguardado: o passo para (AGUARDA) e a fase fica no item 3 do registro.
    fn delegar(&mut self, reg: Valor, modo: u8, v: Valor, assinc: bool) -> Res<Passo> {
        const RESULTADO: i32 = 1;
        const RESULTADO_DO_RETURN: i32 = 2;
        const VALOR_DO_RETURN: i32 = 3;
        const FECHOU_SEM_THROW: i32 = 4;
        let it = self.lista_get(reg, 0);
        let fase = self.lista_get(reg, 3);
        let fase = if fase.e_int() { fase.como_int() } else { 0 };
        self.lista_set(reg, 3, Valor::int(0));
        if fase != 0 && modo == RETOMOU_LANCA {
            // A espera foi rejeitada: o erro sobe no gerador de fora.
            return Err(v);
        }
        let aguardar = |m: &mut Motor, fase: i32, x: Valor| {
            m.lista_set(reg, 3, Valor::int(fase));
            Ok(Passo::Para(x, AGUARDA))
        };
        match fase {
            RESULTADO => return self.delegar_resultado(v, false, assinc),
            RESULTADO_DO_RETURN => return self.delegar_resultado(v, true, assinc),
            VALOR_DO_RETURN => return self.delegar_retorno(reg, v, assinc),
            FECHOU_SEM_THROW => {
                if !self.e_objeto(v) {
                    return self.erro_tipo("Iterator result is not an object");
                }
                return self.erro_tipo("The iterator does not provide a 'throw' method");
            }
            _ => {}
        }
        match modo {
            RETOMOU_NORMAL => {
                let next = self.lista_get(reg, 1);
                let r = self.chamar(next, it, &[v])?;
                if assinc {
                    return aguardar(self, RESULTADO, r);
                }
                self.delegar_resultado(r, false, assinc)
            }
            RETOMOU_LANCA => match self.obter_metodo(it, ChaveProp::Nome(fixos::THROW))? {
                Some(f) => {
                    let r = self.chamar(f, it, &[v])?;
                    if assinc {
                        return aguardar(self, RESULTADO, r);
                    }
                    self.delegar_resultado(r, false, assinc)
                }
                None => {
                    // Sem `throw` o de dentro não recebe o erro: fecha ele e lança TypeError.
                    if let Some(f) = self.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))? {
                        let r = self.chamar(f, it, &[])?;
                        if assinc {
                            return aguardar(self, FECHOU_SEM_THROW, r);
                        }
                        if !self.e_objeto(r) {
                            return self.erro_tipo("Iterator result is not an object");
                        }
                    }
                    self.erro_tipo("The iterator does not provide a 'throw' method")
                }
            },
            _ => {
                // Em async, o valor do return é aguardado antes (AsyncGeneratorUnwrapYieldResumption).
                if assinc {
                    return aguardar(self, VALOR_DO_RETURN, v);
                }
                self.delegar_retorno(reg, v, assinc)
            }
        }
    }

    fn delegar_retorno(&mut self, reg: Valor, v: Valor, assinc: bool) -> Res<Passo> {
        let it = self.lista_get(reg, 0);
        match self.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))? {
            None => Ok(Passo::Retorno(v)),
            Some(f) => {
                let r = self.chamar(f, it, &[v])?;
                if assinc {
                    self.lista_set(reg, 3, Valor::int(2));
                    return Ok(Passo::Para(r, AGUARDA));
                }
                self.delegar_resultado(r, true, assinc)
            }
        }
    }

    fn delegar_resultado(&mut self, r: Valor, de_retorno: bool, assinc: bool) -> Res<Passo> {
        if !self.e_objeto(r) {
            return self.erro_tipo("Iterator result is not an object");
        }
        let feito = self.obter(r, ChaveProp::Nome(fixos::DONE))?;
        if self.verdadeiro(feito) {
            let v = self.obter(r, ChaveProp::Nome(fixos::VALUE))?;
            return Ok(if de_retorno { Passo::Retorno(v) } else { Passo::Fim(v) });
        }
        if assinc {
            let v = self.obter(r, ChaveProp::Nome(fixos::VALUE))?;
            return Ok(Passo::Para(v, RENDEU));
        }
        Ok(Passo::Para(r, RENDEU_CRU))
    }

    /// A instrução onde o quadro do topo parou é uma destas?
    fn parado_em(&self, ops: &[Op]) -> bool {
        let Some(q) = self.quadros.last() else { return false };
        let p = &self.protos[q.proto as usize];
        let codigo = &p.codigo().codigo;
        let mut i = q.pc as usize;
        if matches!(codigo.get(i).copied(), Some(b) if b == Op::Largo as u8 || b == Op::Largo4 as u8) {
            i += 1;
        }
        codigo.get(i).and_then(|&b| Op::de_byte(b)).is_some_and(|op| ops.contains(&op))
    }

    fn iterador_de_array_intacto(&mut self, a: Valor) -> bool {
        if self.proto_de(a) != self.intr.array_proto || self.propria(a, simb::ITERATOR).is_some() {
            return false;
        }
        let ap = self.intr.array_proto;
        let aip = self.intr.array_iterador_proto;
        let f = self.propria(ap, simb::ITERATOR).map(|d| self.ler_campo(ap, d.slot));
        let n = self.propria(aip, fixos::NEXT).map(|d| self.ler_campo(aip, d.slot));
        f == Some(self.intr.array_valores) && n == Some(self.intr.array_iterador_next)
    }

    /// Próximo valor do registro; `None` quando acabou.
    pub fn proximo(&mut self, reg: Valor) -> Res<Option<Valor>> {
        let it = self.lista_get(reg, 0);
        if self.lista_get(reg, 1) == IT_ARRAY {
            let i = self.lista_get(reg, 2);
            if i == Valor::TRUE {
                return Ok(None);
            }
            let i = i.como_int() as u32;
            if i >= self.array_len(it) {
                self.lista_set(reg, 2, Valor::TRUE);
                return Ok(None);
            }
            self.lista_set(reg, 2, Valor::int(i as i32 + 1));
            return match self.elemento(it, i) {
                Some(v) => Ok(Some(v)),
                None => self.obter(it, ChaveProp::Indice(i)).map(Some),
            };
        }
        if self.lista_get(reg, 2) == Valor::TRUE {
            return Ok(None);
        }
        let next = self.lista_get(reg, 1);
        let r = match self.chamar(next, it, &[]) {
            Ok(r) => r,
            Err(e) => {
                self.lista_set(reg, 2, Valor::TRUE);
                return Err(e);
            }
        };
        if !self.e_objeto(r) {
            self.lista_set(reg, 2, Valor::TRUE);
            return self.erro_tipo("Iterator result is not an object");
        }
        let feito = self.obter(r, ChaveProp::Nome(fixos::DONE))?;
        if self.verdadeiro(feito) {
            self.lista_set(reg, 2, Valor::TRUE);
            return Ok(None);
        }
        self.obter(r, ChaveProp::Nome(fixos::VALUE)).map(Some)
    }

    pub fn fechar_iterador(&mut self, reg: Valor) -> Res<()> {
        if self.lista_get(reg, 1) == IT_ARRAY || self.lista_get(reg, 2) == Valor::TRUE {
            return Ok(());
        }
        self.lista_set(reg, 2, Valor::TRUE);
        let it = self.lista_get(reg, 0);
        if let Some(f) = self.obter_metodo(it, ChaveProp::Nome(fixos::RETURN))? {
            self.chamar(f, it, &[])?;
        }
        Ok(())
    }

    /// Chama `f` com cada valor de um iterável (e o índice), um por vez, sem juntar tudo antes:
    /// um iterador infinito com callback que lança não enche a memória. Se `f` falha, fecha o
    /// iterador (IteratorClose) e devolve o erro de `f`.
    pub fn percorrer_iteravel(&mut self, v: Valor, mut f: impl FnMut(&mut Motor, Valor, u64) -> Res<()>) -> Res<()> {
        let reg = self.obter_iterador(v)?;
        let mut i = 0;
        while let Some(x) = self.proximo(reg)? {
            if let Err(e) = f(self, x, i) {
                let _ = self.fechar_iterador(reg);
                return Err(e);
            }
            i += 1;
        }
        Ok(())
    }

    /// Todos os valores de um iterável.
    pub fn coletar_iteravel(&mut self, v: Valor) -> Res<Vec<Valor>> {
        let reg = self.obter_iterador(v)?;
        let mut r = Vec::new();
        while let Some(x) = self.proximo(reg)? {
            r.push(x);
        }
        Ok(r)
    }

    // ---------------------------------------------------------------- for-in

    /// Estado do for-in: lista `[objeto, próximo índice, chaves…]`.
    fn preparar_for_in(&mut self, v: Valor) -> Res<Valor> {
        if v.e_nulo_ou_indefinido() {
            let l = self.nova_lista(2);
            self.lista_set(l, 0, Valor::NULL);
            self.lista_set(l, 1, Valor::int(2));
            return Ok(l);
        }
        let o = self.para_objeto(v)?;
        let mut vistas: std::collections::HashSet<ChaveProp> = Default::default();
        let mut chaves = Vec::new();
        let mut p = o;
        while p != Valor::NULL {
            // Proxy: as chaves e a enumerabilidade vêm das armadilhas (`ownKeys`,
            // `getOwnPropertyDescriptor`), que podem lançar.
            let proxy = self.formas.get(self.forma(p)).classe == Classe::Proxy;
            let ks = if proxy { self.chaves_proprias_res(p)? } else { self.chaves_de(p, false, true, false) };
            for k in ks {
                if matches!(k, ChaveProp::Nome(c) if c.e_simbolo()) || !vistas.insert(k) {
                    continue;
                }
                let d = if proxy { self.descritor_proprio_res(p, k)? } else { self.descritor_proprio(p, k) };
                let enumeravel = d.is_some_and(|d| d.attrs & attr::ENUMERAVEL != 0);
                if enumeravel {
                    chaves.push(k);
                }
            }
            p = self.proto_de(p);
        }
        let l = self.nova_lista(2 + chaves.len() as u32);
        self.lista_set(l, 0, o);
        self.lista_set(l, 1, Valor::int(2));
        for (i, k) in chaves.into_iter().enumerate() {
            let kv = self.chave_para_valor(k);
            self.lista_set(l, 2 + i as u32, kv);
        }
        Ok(l)
    }

    fn proximo_for_in(&mut self, st: Valor) -> Res<Option<Valor>> {
        let o = self.lista_get(st, 0);
        loop {
            let i = self.lista_get(st, 1).como_int() as u32;
            if i >= self.lista_len(st) {
                return Ok(None);
            }
            self.lista_set(st, 1, Valor::int(i as i32 + 1));
            let k = self.lista_get(st, i);
            // Apagada no meio do laço: pula.
            let kp = self.para_chave(k)?;
            if self.tem(o, kp)? {
                return Ok(Some(k));
            }
        }
    }

    // ---------------------------------------------------------------- caches de propriedade

    /// Entrada do cache `ci` para a forma `fo`: a monomórfica, uma das polimórficas ou a da
    /// tabela megamórfica.
    #[inline(always)]
    fn entrada_do_cache(&self, proto: usize, ci: u32, fo: u32, c: Chave) -> Option<CacheProp> {
        let cache = self.protos[proto].caches[ci as usize];
        if cache.forma == fo {
            return Some(cache);
        }
        match cache.poli {
            SEM_POLI => None,
            MEGA => self.mega.get(&(fo, c)).copied(),
            i => self.poli[i as usize].iter().find(|e| e.forma == fo).copied(),
        }
    }

    /// Guarda `e` no cache `ci`: na entrada monomórfica se está vazia (ou é da mesma forma), nas
    /// polimórficas até 4 formas, e na tabela megamórfica depois.
    fn guardar_no_cache(&mut self, proto: usize, ci: u32, c: Chave, e: CacheProp) {
        let cache = self.protos[proto].caches[ci as usize];
        if cache.forma == u32::MAX || cache.forma == e.forma {
            self.protos[proto].caches[ci as usize] = CacheProp { poli: cache.poli, ..e };
            return;
        }
        match cache.poli {
            SEM_POLI => {
                let vazio = CacheProp::default();
                self.protos[proto].caches[ci as usize].poli = self.poli.len() as u32;
                self.poli.push([e, vazio, vazio]);
            }
            MEGA => self.guardar_mega(c, e),
            i => {
                let p = &mut self.poli[i as usize];
                if let Some(s) = p.iter_mut().find(|x| x.forma == u32::MAX || x.forma == e.forma) {
                    *s = e;
                } else {
                    self.protos[proto].caches[ci as usize].poli = MEGA;
                    self.guardar_mega(c, e);
                }
            }
        }
    }

    fn guardar_mega(&mut self, c: Chave, e: CacheProp) {
        // shortcut: a tabela é zerada ao passar de 8192 entradas (formas nunca morrem); trocar
        // por tabela de tamanho fixo com substituição se algum site real encher.
        if self.mega.len() >= 8192 {
            self.mega.clear();
        }
        self.mega.insert((e.forma, c), e);
    }

    fn ler_prop_cache(&mut self, proto: usize, ci: u32, o: Valor, c: Chave) -> Res<Valor> {
        if o.e_ref() {
            let fo = self.forma(o).0;
            if let Some(cache) = self.entrada_do_cache(proto, ci, fo, c) {
                if cache.dono == Valor::UNDEFINED {
                    return Ok(self.ler_campo(o, cache.deslocamento));
                }
                if self.forma(cache.dono).0 == cache.forma_dono {
                    return Ok(self.ler_campo(cache.dono, cache.deslocamento));
                }
            }
        }
        // `arr.length` (a condição de quase todo laço) não mora na forma: lê o campo interno.
        if c == fixos::LENGTH && self.e_array(o) {
            let n = self.array_len(o);
            return Ok(self.numero(f64::from(n)));
        }
        if c == fixos::LENGTH && self.e_objeto(o) && self.formas.get(self.forma(o)).classe == Classe::ArrayTipado {
            // shortcut: não confere se alguém trocou o getter `length` no protótipo; conferir se aparecer site que troca.
            return Ok(Valor::int(self.tipado_len(o) as i32));
        }
        let v = self.obter(o, ChaveProp::Nome(c))?;
        // Preenche o cache: própria de dados, ou de dados no protótipo direto.
        if o.e_ref() && !(c == fixos::LENGTH && !self.e_objeto(o)) {
            let fo = self.forma(o);
            let objeto = self.e_objeto(o);
            if objeto && self.formas.get(fo).dicionario {
                return Ok(v);
            }
            let especial_len = c == fixos::LENGTH && objeto && matches!(self.formas.get(fo).classe, Classe::Array | Classe::Caixa);
            if especial_len || self.formas.get(fo).classe == Classe::Proxy {
                return Ok(v);
            }
            let propria = if objeto { self.propria(o, c) } else { None };
            let cp = if let Some(d) = propria {
                (d.attrs & (attr::ACESSOR | attr::PREGUICOSO) == 0).then_some((Valor::UNDEFINED, 0, d.slot))
            } else {
                let p = if objeto { self.proto_de(o) } else { self.proto_do_primitivo(o) };
                if p != Valor::NULL && !self.formas.get(self.forma(p)).dicionario {
                    self.propria(p, c).filter(|d| d.attrs & (attr::ACESSOR | attr::PREGUICOSO) == 0).map(|d| (p, self.forma(p).0, d.slot))
                } else {
                    None
                }
            };
            if let Some((dono, forma_dono, slot)) = cp {
                self.guardar_no_cache(proto, ci, c, CacheProp { forma: fo.0, deslocamento: slot, dono, forma_dono, poli: SEM_POLI });
            }
        }
        Ok(v)
    }

    fn gravar_prop_cache(&mut self, proto: usize, ci: u32, o: Valor, c: Chave, v: Valor, estrito: bool) -> Res<()> {
        if o.e_ref() {
            let fo = self.forma(o).0;
            if let Some(cache) = self.entrada_do_cache(proto, ci, fo, c) {
                if cache.dono == Valor::UNDEFINED {
                    self.gravar_campo(o, cache.deslocamento, v);
                    return Ok(());
                }
            }
        }
        self.gravar(o, ChaveProp::Nome(c), v, estrito)?;
        if self.e_objeto(o) {
            let fo = self.forma(o);
            if !self.formas.get(fo).dicionario && !(c == fixos::LENGTH && self.e_array(o)) {
                if let Some(d) = self.propria(o, c) {
                    if d.attrs & (attr::ACESSOR | attr::GRAVAVEL) == attr::GRAVAVEL {
                        self.guardar_no_cache(proto, ci, c, CacheProp { forma: fo.0, deslocamento: d.slot, dono: Valor::UNDEFINED, forma_dono: u32::MAX, poli: SEM_POLI });
                    }
                }
            }
        }
        Ok(())
    }
}


#[cfg(test)]
mod testes_compilar_texto {
    use crate::Motor;

    #[test]
    fn embutidor_recusa_eval_e_function() {
        let mut m = Motor::novo();
        m.ao_compilar_texto = Some(|_| Err("CSP".to_owned()));
        let v = m
            .avaliar(
                "const r = [];
                 for (const f of [() => eval('1'), () => (0, eval)('1'), () => new Function('return 1')]) {
                   try { f(); r.push('rodou'); } catch (e) { r.push(e.name + ':' + e.message); }
                 }
                 r.join(',')",
            )
            .unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        assert_eq!(m.para_texto_rust(v).unwrap(), "EvalError:CSP,EvalError:CSP,EvalError:CSP");
    }

    /// `!function f(b){ … f(c) … }` do jQuery (Callbacks.add): o nome capturado por função
    /// interna, com outro `f` de fora, era lido do registrador errado ("object is not a function").
    #[test]
    fn nome_de_funcao_capturado_por_funcao_interna() {
        let mut m = Motor::novo();
        let v = m
            .avaliar(
                "var f = {}, out = [];
                 !function f(b){ Array.prototype.forEach.call(b, function(c){ typeof c == 'number' ? out.push(c) : f(c); }); }([1, [2, [3]], 4]);
                 var g = function h(n){ return n ? (function(){ return h(n - 1) + 1; })() : 0; };
                 out.join(',') + '|' + g(5) + '|' + typeof f",
            )
            .unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        assert_eq!(m.para_texto_rust(v).unwrap(), "1,2,3,4|5|object");
    }

    #[test]
    fn chave_numerica_de_objeto_literal_e_indice() {
        // Carregador do browserify (fast.com): `{1:[…],3:[…]}` lido com `e[3]`.
        let mut m = Motor::novo();
        let v = m
            .avaliar(
                "var e = {1: 'a', '3': 'b', 07: 'c', '07': 'd', 4294967295: 'e', x: function(){}, 2: function(){}};
                 var {3: t} = e;
                 [e[1], e['1'], e[3], t, e[7], e['07'], e[4294967295], Object.keys(e).join(), e[2].name].join('|')",
            )
            .unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        assert_eq!(m.para_texto_rust(v).unwrap(), "a|a|b|b|c|d|e|1,2,3,7,07,4294967295,x|2");
    }
}

#[cfg(test)]
mod testes_pilha {
    use crate::Motor;

    /// A pilha dos erros diz a fonte, a linha e a coluna (a partir de 1) de cada quadro.
    #[test]
    fn pilha_tem_origem_linha_e_coluna() {
        let mut m = Motor::novo();
        let e = m.avaliar_com_nome("log = 1;\nfunction g() {\n  throw new Error('x');\n}\ng();", "macro").unwrap_err();
        let t = m.texto_do_erro(e);
        assert!(t.contains("at g (macro:3:"), "{t}");
        assert!(t.contains("(macro:5:"), "{t}");
        let e = m.avaliar("\n\nthrow new Error('y');").unwrap_err();
        let t = m.texto_do_erro(e);
        assert!(t.contains("(:3:"), "sem nome, só a linha e a coluna: {t}");
        // Erro do próprio motor no meio de uma função: a linha é a da instrução, não a da entrada.
        let e = m.avaliar_com_nome("function f(o) {\n  let x = 1;\n  return o.a;\n}\nf(null);", "macro").unwrap_err();
        let t = m.texto_do_erro(e);
        assert!(t.contains("at f (macro:3:"), "{t}");
        let e = m.avaliar("let a = 1;\nlet b = ;").unwrap_err();
        let t = m.texto_do_erro(e);
        assert!(t.contains("(linha 2, coluna 9)"), "{t}");
    }
}

#[cfg(test)]
mod testes_entrada_hostil {
    use crate::Motor;

    /// Script de página não derruba o processo: cada caso vira erro JS (a thread de teste tem
    /// pilha de 2 MB, então estouro de pilha aqui aborta o teste).
    #[test]
    fn entrada_hostil_vira_erro_js() {
        let fundo = |abre: &str, meio: &str, fecha: &str| format!("x = {}{meio}{};", abre.repeat(100_000), fecha.repeat(100_000));
        let mut m = Motor::novo();
        for fonte in [fundo("[", "", "]"), fundo("(", "1", ")"), fundo("", "1", "**1"), fundo("", "o", ".o"), fundo("function a(){", "", "}")] {
            let e = m.avaliar(&fonte).unwrap_err();
            assert!(m.texto_do_erro(e).contains("SyntaxError"), "{}", &fonte[..20]);
        }
        // Cadeia funda à esquerda (texto montado com `+`) compila num laço: não é erro.
        let v = m.avaliar(&fundo("", "1", "+1")).unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        assert_eq!(m.para_texto_rust(v).unwrap(), "100001");
        let v = m
            .avaliar(
                "const r = [];
                 const casos = [
                   () => { const o = { get x() { return this.x; } }; o.x; },
                   () => { const o = { toString() { return '' + o; } }; '' + o; },
                   () => { function f() { return [1].map(f); } f(); },
                   () => { const p = new Proxy({}, { get(t, k, r) { return r[k]; } }); p.x; },
                   () => { let s = 'ab'; for (;;) s = s + s; },
                   () => new RegExp('('.repeat(100000) + ')'.repeat(100000)),
                   () => new ArrayBuffer(4294967295),
                   () => { let p = {x: 1}; for (let i = 0; i < 100000; i++) p = new Proxy(p, {}); p.x; },
                   () => { let f = function () {}; for (let i = 0; i < 100000; i++) f = f.bind(null); new f(); },
                   () => { let o = {}; for (let i = 0; i < 100000; i++) o = {o}; JSON.stringify(o); },
                   () => { let a = []; for (let i = 0; i < 100000; i++) a = [a]; a.flat(Infinity); },
                   () => 2n ** (2n ** 24n),
                   () => BigInt('9'.repeat(1e7)),
                 ];
                 for (const f of casos) {
                   try { f(); r.push('rodou'); } catch (e) { r.push(e.name); }
                 }
                 const a = [1, 2];
                 a.length = 4294967295;
                 try { a.push(7, 8); } catch (e) { r.push(e.name); }
                 r.push(a[0]);
                 r.join(',')",
            )
            .unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        assert_eq!(
            m.para_texto_rust(v).unwrap(),
            "RangeError,RangeError,RangeError,RangeError,RangeError,SyntaxError,RangeError,RangeError,RangeError,RangeError,RangeError,RangeError,SyntaxError,RangeError,1"
        );
    }
}

#[cfg(test)]
mod testes_perfil {
    use crate::Motor;

    /// Caches de propriedade com 1, 2-4 e 5+ formas leem o certo (inclusive do protótipo, depois
    /// de o protótipo mudar e de o coletor mover tudo), e o perfil anota tipos e calor.
    #[test]
    fn caches_polimorficos_e_perfil() {
        let mut m = Motor::novo();
        m.estresse = true;
        let v = m
            .avaliar(
                "function ler(o) { return o.v; }
                 function gravar(o, x) { o.v = x; return o.v; }
                 const base = {v: 'proto'};
                 const formas = [{v: 1}, {a: 1, v: 2}, {b: 1, v: 3}, {c: 1, v: 4}, {d: 1, v: 5}, {e: 1, v: 6}, Object.create(base)];
                 let r = [];
                 for (let k = 0; k < 3; k++) for (const o of formas) { r.push(ler(o)); r.push(gravar({w: 0, v: 0}, k)); }
                 base.v = 'mudou';
                 Object.defineProperty(base, 'outra', {value: 1});
                 r.push(ler(formas[6]));
                 function conta(a, b) { return a + b; }
                 conta(1, 2); conta(1.5, 2); conta('a', 1); conta(2 ** 29, 2 ** 29);
                 r.join()",
            )
            .unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        let uma = "1,0,2,0,3,0,4,0,5,0,6,0,proto,0";
        let esperado = format!("{uma},{},{},mudou", uma.replace(",0", ",1"), uma.replace(",0", ",2"));
        assert_eq!(m.para_texto_rust(v).unwrap(), esperado);
        let r = m.relatorio_perfil();
        assert!(r.contains("ler: 22 chamadas") && r.contains("megamórfico"), "{r}");
        assert!(r.contains("int|double|texto|estourou"), "{r}");
    }
}
