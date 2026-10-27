//! Intérprete N1 (`ARQUITETURA.md`, seção 9): laço sobre o bytecode, com os quadros numa pilha
//! própria. Chamada JS→JS não usa a pilha do Rust: empurra um [`Quadro`] e continua no mesmo
//! laço. Nativas que chamam JS (`chamar`) abrem um laço novo, que termina quando o quadro de
//! entrada volta.
//!
//! Registradores do quadro moram em `Motor::pilha[base..fim]`; os parâmetros são os primeiros.
//! Se vieram mais argumentos que parâmetros, eles ficam logo antes de `base` (para `arguments`).

use crate::formas::{attr, Chave, Classe};
use crate::funcoes::{Args, ConstRt, Res, BIT_NATIVA, F_CASA, F_CODIGO, F_CONTEXTO, F_THIS};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::operadores::{Arit, Bits};
use crate::textos::ChaveProp;
use crate::valor::Valor;
use falcao_bytecode::instrucoes::{ler_operando, Operando};
use falcao_bytecode::{Constante, Op};
use std::rc::Rc;

/// Sem gerador/async ligado ao quadro.
pub const SEM_SUSPENSO: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
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
    /// Promessa da função async (undefined em gerador).
    pub promessa: Valor,
}

/// Como um laço termina.
#[derive(Clone, Copy, Debug)]
pub enum Fim {
    Retorno(Valor),
    Suspenso(Valor),
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

    /// Busca dinâmica (`with`): os objetos de `with` da corrente, depois o global.
    fn ler_dinamico(&mut self, mut ctx: Valor, c: Chave, typeof_: bool) -> Res<Valor> {
        let k = ChaveProp::Nome(c);
        while ctx != Valor::UNDEFINED {
            if self.g.u32_em(ctx.deslocamento() + 4) & CTX_COM != 0 {
                let o = self.ctx_ler(ctx, 0);
                if self.tem(o, k)? {
                    return self.obter(o, k);
                }
            }
            ctx = self.ctx_pai(ctx);
        }
        self.ler_global(c, typeof_)
    }

    fn gravar_dinamico(&mut self, mut ctx: Valor, c: Chave, v: Valor, estrito: bool) -> Res<()> {
        let k = ChaveProp::Nome(c);
        while ctx != Valor::UNDEFINED {
            if self.g.u32_em(ctx.deslocamento() + 4) & CTX_COM != 0 {
                let o = self.ctx_ler(ctx, 0);
                if self.tem(o, k)? {
                    return self.gravar(o, k, v, estrito);
                }
            }
            ctx = self.ctx_pai(ctx);
        }
        self.gravar_global(c, v, estrito)
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
        let fb = self.protos[proto as usize].codigo();
        let (np, nregs, fl) = (fb.parametros, fb.registradores, fb.flags);
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
        let topo = self.pilha.len() as u32;
        let (base, args_q) = if args.n <= np {
            self.pilha.extend_from_within(args.base as usize..(args.base + args.n) as usize);
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
        self.protos[proto as usize].chamadas += 1;
        Ok(())
    }

    /// Chama `f`. `Some(v)`: terminou (nativa); `None`: empurrou um quadro de bytecode.
    pub(crate) fn entrar(&mut self, f: Valor, this: Valor, args: Args) -> Res<Option<Valor>> {
        if !self.e_objeto(f) {
            return self.nao_e_funcao(f);
        }
        match self.formas.get(self.forma(f)).classe {
            Classe::Funcao => {
                let c = self.interno(f, F_CODIGO);
                if c & BIT_NATIVA != 0 {
                    return self.chamar_nativa(c & !BIT_NATIVA, f, this, args, Valor::UNDEFINED).map(Some);
                }
                let fl = self.protos[c as usize].codigo().flags;
                if fl.construtor_classe {
                    return self.erro_tipo("Class constructor cannot be invoked without 'new'");
                }
                if fl.assincrona {
                    if fl.gerador {
                        return self.erro_tipo("geradores assíncronos ainda não são suportados no Falcão");
                    }
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
        let antes = self.chamado;
        self.chamado = f;
        self.aninhamento += 1;
        let r = nf(self, this, args, nt);
        self.aninhamento -= 1;
        self.chamado = antes;
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

    /// Inicia os campos de instância da classe `f` em `o`.
    pub fn rodar_campos(&mut self, f: Valor, o: Valor) -> Res<()> {
        let Some(d) = self.propria(f, simb::CAMPOS) else { return Ok(()) };
        let lista = self.ler_campo(f, d.slot);
        let n = self.lista_len(lista);
        let mut i = 0;
        while i + 1 < n {
            let (k, init) = (self.lista_get(lista, i), self.lista_get(lista, i + 1));
            let v = if init == Valor::UNDEFINED { Valor::UNDEFINED } else { self.chamar(init, o, &[])? };
            let k = self.para_chave(k)?;
            match k {
                ChaveProp::Nome(c) if c.e_simbolo() && self.escondidos.contains(&(c.0 & !Chave::BIT_SIMBOLO)) => {
                    self.definir_propria(o, c, v, attr::GRAVAVEL);
                }
                _ => {
                    if !self.criar_dado(o, k, v)? {
                        return self.erro_tipo("Cannot define class field");
                    }
                }
            }
            i += 2;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- chamadas aninhadas

    fn empurrar_args(&mut self, args: &[Valor]) -> Args {
        let base = self.pilha.len() as u32;
        self.pilha.extend_from_slice(args);
        Args { base, n: args.len() as u32 }
    }

    /// Chama `f(this, ...args)` de dentro do motor (nativas, conversões).
    pub fn chamar(&mut self, f: Valor, this: Valor, args: &[Valor]) -> Res<Valor> {
        self.aninhamento += 1;
        let r = self.chamar_externo(f, this, args);
        self.aninhamento -= 1;
        r
    }

    /// Chamada vinda de fora (o embutidor). Não pode haver `Valor` solto no Rust durante ela
    /// além dos registrados em raízes: o coletor pode rodar.
    pub fn chamar_externo(&mut self, f: Valor, this: Valor, args: &[Valor]) -> Res<Valor> {
        let marca = self.pilha.len();
        let a = self.empurrar_args(args);
        let r = match self.entrar(f, this, a) {
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
        self.pilha.truncate(marca);
        r
    }

    /// `new f(...args)` de dentro do motor.
    pub fn construir(&mut self, f: Valor, args: &[Valor], nt: Valor) -> Res<Valor> {
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
                    Ok(s) => s,
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
        QuadroSalvo { quadro: q, valores, promessa: Valor::UNDEFINED }
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
                let espera = self.promessa_de(v);
                self.reagir_async(espera, id);
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
            if let Some(x) = fb.excecoes.iter().find(|x| x.inicio <= pc && pc < x.fim).copied() {
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
        match ent {
            Entrada::Normal(v) => acc = v,
            Entrada::Lanca(e) => pendente = Some((e, false)),
            Entrada::Retorna(v) => {
                let q = self.quadros.pop().unwrap();
                self.pilha.truncate(q.topo as usize);
                return Ok(Fim::Retorno(v));
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
            if self.g.coleta_pendente && self.aninhamento == 0 {
                acc = self.ponto_seguro(acc);
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
                        let v = i32::from_le_bytes([codigo[pc], codigo[pc + 1], codigo[pc + 2], codigo[pc + 3]]);
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
                    Op::CarregaInt => acc = Valor::int(i4!()),
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
                        let g = self.intr.global;
                        let cache = self.protos[proto].caches[ci as usize];
                        if cache.forma == self.forma(g).0 && cache.forma_dono == self.epoca_lexico {
                            acc = self.ler_campo(g, cache.deslocamento);
                        } else {
                            acc = t!(self.ler_global(c, op == Op::LeGlobalTypeof));
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
                        }
                    }
                    Op::GravaGlobal => {
                        let (k, ci) = (u!(), u!());
                        let c = nome!(k);
                        let g = self.intr.global;
                        let cache = self.protos[proto].caches[ci as usize];
                        if cache.forma == self.forma(g).0 && cache.forma_dono == self.epoca_lexico {
                            self.gravar_campo(g, cache.deslocamento, acc);
                        } else {
                            t!(self.gravar_global(c, acc, estrito));
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
                        }
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
                        if acc.e_int() && acc.como_int() >= 0 && self.e_objeto(o) {
                            if let Some(v) = self.elemento(o, acc.como_int() as u32) {
                                acc = v;
                                continue 'exec;
                            }
                            if self.formas.get(self.forma(o)).classe == Classe::ArrayTipado {
                                acc = self.tipado_ler(o, acc.como_int() as u32).unwrap_or(Valor::UNDEFINED);
                                continue 'exec;
                            }
                        }
                        if !self.e_objeto(o) && o.e_nulo_ou_indefinido() {
                            let k = t!(self.para_chave(acc));
                            acc = t!(self.obter(o, k));
                        } else {
                            let k = t!(self.para_chave(acc));
                            acc = t!(self.obter(o, k));
                        }
                    }
                    Op::GravaItem => {
                        let (r0, r1, _ci) = (u!(), u!(), u!());
                        let (o, kv) = (reg!(r0), reg!(r1));
                        if kv.e_int() && kv.como_int() >= 0 && self.e_objeto(o) {
                            let i = kv.como_int() as u32;
                            let classe = self.formas.get(self.forma(o)).classe;
                            if classe == Classe::ArrayTipado {
                                t!(self.tipado_gravar(o, i, acc));
                                continue 'exec;
                            }
                            // Item que já está na lista é comum (os especiais saem dela); item novo só sem índices especiais.
                            let existe = self.elemento(o, i).is_some();
                            if matches!(classe, Classe::Array | Classe::Comum | Classe::Argumentos) && (existe || (!self.indices_especiais && (self.proto_de(o) == self.intr.array_proto || self.proto_de(o) == self.intr.objeto_proto))) {
                                if self.gravar_elemento(o, i, acc) {
                                    continue 'exec;
                                }
                            }
                        }
                        let k = t!(self.para_chave(kv));
                        t!(self.gravar(o, k, acc, estrito));
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
                        let (r, k) = (u!(), u!());
                        let o = reg!(r);
                        let c = self.chave_privada(nome!(k));
                        if !self.e_objeto(o) || !t!(self.tem(o, ChaveProp::Nome(c))) {
                            t!(self.erro_tipo::<()>("Cannot read private member from an object whose class did not declare it"));
                        }
                        acc = t!(self.obter(o, ChaveProp::Nome(c)));
                    }
                    Op::GravaPrivado => {
                        let (r, k) = (u!(), u!());
                        let o = reg!(r);
                        let c = self.chave_privada(nome!(k));
                        if !self.e_objeto(o) || !t!(self.tem(o, ChaveProp::Nome(c))) {
                            t!(self.erro_tipo::<()>("Cannot write private member to an object whose class did not declare it"));
                        }
                        t!(self.gravar(o, ChaveProp::Nome(c), acc, true));
                    }
                    Op::TemPrivado => {
                        let k = u!();
                        let c = self.chave_privada(nome!(k));
                        if !self.e_objeto(acc) {
                            t!(self.erro_tipo::<()>("Cannot use 'in' operator to search for a private field in a non-object"));
                        }
                        acc = Valor::bool(t!(self.tem(acc, ChaveProp::Nome(c))));
                    }
                    Op::DefinePrivado => {
                        let (r, k) = (u!(), u!());
                        let c = self.chave_privada(nome!(k));
                        self.definir_propria(reg!(r), c, acc, attr::GRAVAVEL);
                    }
                    Op::ChavePrivada => {
                        let c = self.chave_privada(nome!(u!()));
                        acc = self.simbolos[(c.0 & !Chave::BIT_SIMBOLO) as usize];
                    }

                    // ---- literais ----
                    Op::CriaObjeto => {
                        let n = u!();
                        let f = if n <= 4 {
                            self.intr.forma_objeto
                        } else {
                            let op = self.intr.objeto_proto;
                            self.forma_para(Classe::Comum, op, n.min(64))
                        };
                        acc = self.novo_objeto(f);
                    }
                    Op::CriaArray => {
                        let _ = u!();
                        let f = self.intr.forma_array;
                        acc = self.novo_objeto(f);
                    }
                    Op::DefProp => {
                        let (r, k) = (u!(), u!());
                        let c = nome!(k);
                        self.definir_propria(reg!(r), c, acc, attr::PADRAO);
                    }
                    Op::DefItem => {
                        let (r0, r1) = (u!(), u!());
                        let k = t!(self.para_chave(reg!(r1)));
                        let o = reg!(r0);
                        if let ChaveProp::Nome(c) = k {
                            self.definir_propria(o, c, acc, attr::PADRAO);
                        } else {
                            t!(self.criar_dado(o, k, acc));
                        }
                    }
                    Op::Empurra => {
                        let a = reg!(u!());
                        let n = self.array_len(a);
                        self.gravar_elemento(a, n, acc);
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
                        let ConstRt::Funcao(p) = self.protos[proto].consts[k as usize] else { panic!("CriaFuncao sem função") };
                        let q = self.quadros[qi];
                        let this = if fb.flags.seta { Valor(self.interno(q.funcao, F_THIS)) } else { q.this };
                        let f = self.closure(p, q.contexto, this, None);
                        if fb.flags.seta || self.protos[p as usize].codigo().flags.seta {
                            // A seta herda a casa (super) de quem a cria.
                            let casa = Valor(self.interno(q.funcao, F_CASA));
                            if q.funcao != Valor::UNDEFINED && casa != Valor::UNDEFINED {
                                self.por_interno_valor(f, F_CASA, casa);
                            }
                        }
                        acc = f;
                    }
                    Op::CriaClasse => {
                        let (k, rh) = (u!(), u!());
                        let ctx = self.quadros[qi].contexto;
                        acc = t!(self.criar_classe(proto, k, reg!(rh), ctx));
                    }
                    Op::DefCampo => {
                        let (r0, r1) = (u!(), u!());
                        let (classe, kv) = (reg!(r0), reg!(r1));
                        self.registrar_campo(classe, kv, acc);
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
                        let (r, _c) = (u!(), u!());
                        let a = reg!(r);
                        acc = match (a.e_int() && acc.e_int()).then(|| a.como_int().checked_add(acc.como_int())).flatten().and_then(|s| Valor::int_se_cabe(i64::from(s))) {
                            Some(v) => v,
                            None => t!(self.somar(a, acc)),
                        };
                    }
                    Op::Sub | Op::Mul | Op::Div | Op::Resto | Op::Potencia => {
                        let (r, _c) = (u!(), u!());
                        let a = match op {
                            Op::Sub => Arit::Sub,
                            Op::Mul => Arit::Mul,
                            Op::Div => Arit::Div,
                            Op::Resto => Arit::Resto,
                            _ => Arit::Potencia,
                        };
                        acc = t!(self.aritmetica(a, reg!(r), acc));
                    }
                    Op::E | Op::Ou | Op::OuExclusivo | Op::DeslEsq | Op::DeslDir | Op::DeslDirSemSinal => {
                        let (r, _c) = (u!(), u!());
                        let b = match op {
                            Op::E => Bits::E,
                            Op::Ou => Bits::Ou,
                            Op::OuExclusivo => Bits::Xor,
                            Op::DeslEsq => Bits::Esq,
                            Op::DeslDir => Bits::Dir,
                            _ => Bits::DirSemSinal,
                        };
                        acc = t!(self.bits(b, reg!(r), acc));
                    }
                    Op::Igual | Op::Diferente => {
                        let (r, _c) = (u!(), u!());
                        let i = t!(self.iguais(reg!(r), acc));
                        acc = Valor::bool(i == (op == Op::Igual));
                    }
                    Op::Identico | Op::NaoIdentico => {
                        let r = u!();
                        let i = self.identicos(reg!(r), acc);
                        acc = Valor::bool(i == (op == Op::Identico));
                    }
                    Op::Menor | Op::Maior | Op::MenorIgual | Op::MaiorIgual => {
                        let (r, _c) = (u!(), u!());
                        let (a, b) = (reg!(r), acc);
                        if a.e_int() && b.e_int() {
                            let (x, y) = (a.como_int(), b.como_int());
                            acc = Valor::bool(match op {
                                Op::Menor => x < y,
                                Op::Maior => x > y,
                                Op::MenorIgual => x <= y,
                                _ => x >= y,
                            });
                            continue 'exec;
                        }
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
                        let _c = u!();
                        acc = t!(self.somar_int(acc, i));
                    }
                    Op::Negativo => {
                        let _c = u!();
                        acc = t!(self.negativo(acc));
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
                        let _c = u!();
                        acc = t!(self.til(acc));
                    }
                    Op::Typeof => acc = self.typeof_(acc),
                    Op::Incrementa | Op::Decrementa => {
                        let _c = u!();
                        let d = if op == Op::Incrementa { 1 } else { -1 };
                        acc = if self.e_bigint(acc) {
                            let x = self.ler_bigint(acc).somar(&crate::grande::Grande::de_i64(d));
                            self.novo_bigint(&x)
                        } else {
                            let n = if acc.e_int() { acc } else { t!(self.para_numero_valor(acc)) };
                            t!(self.somar_int(n, d as i32))
                        };
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
                        self.voltas = self.voltas.wrapping_add(1);
                        if self.voltas & 0xFFFF == 0 {
                            if let Some(motivo) = self.interromper.as_mut().and_then(|f| f()) {
                                let e = self.erro(crate::intrinsecos::TipoErro::Erro, &motivo);
                                t!(Err::<Valor, Valor>(e));
                            }
                        }
                        if self.g.coleta_pendente && self.aninhamento == 0 {
                            salvar_pc!();
                            acc = self.ponto_seguro(acc);
                        }
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
                        acc = t!(self.super_chamar(qi, &args));
                    }
                    Op::Retorna => {
                        let q = self.quadros[qi];
                        let mut v = acc;
                        if q.construindo && !self.e_objeto(v) {
                            if v != Valor::UNDEFINED && fb.flags.classe_derivada {
                                t!(self.erro_tipo::<()>("Derived constructors may only return object or undefined"));
                            }
                            v = q.this;
                            if v == Valor::BURACO {
                                t!(self.erro_referencia::<()>("Must call super constructor in derived class before accessing 'this' or returning from derived constructor"));
                            }
                        }
                        self.quadros.pop();
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
                        let (r, _n) = (u!(), u!());
                        // shortcut: `for await` e `yield*` assíncrono usam o iterador comum.
                        let reg_it = t!(self.obter_iterador(acc));
                        reg!(r) = reg_it;
                        acc = Valor::UNDEFINED;
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
                        let f = self.quadros[qi].funcao;
                        let p = t!(self.obter(f, ChaveProp::Nome(fixos::PROTOTYPE)));
                        let p = if self.e_objeto(p) { p } else { self.intr.gerador_proto };
                        let forma = self.forma_para(Classe::Gerador, p, 0);
                        let gen = self.novo_objeto(forma);
                        let id = self.novo_suspenso();
                        self.por_interno(gen, 0, id);
                        self.por_interno_valor(gen, 2, f);
                        let entrada = self.quadros[qi].entrada;
                        self.quadros[qi].suspenso = id;
                        let salvo = self.salvar_quadro(pc as u32);
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
                        let salvo = self.salvar_quadro(pc as u32);
                        self.suspensos[id as usize] = Some(salvo);
                        return Ok(Fim::Suspenso(acc));
                    }
                    Op::CriaResultado => {
                        let n = u!();
                        acc = self.resultado_iterador(acc, n == 1);
                    }
                    Op::ModoRetomada => {
                        let _ = u!();
                    }
                    Op::Delega => {
                        let r = u!();
                        let a = i4!() as u32 as usize;
                        let it = reg!(r);
                        salvar_pc!();
                        // shortcut: throw/return no gerador de fora não repassam ao de dentro.
                        let res = t!(self.passo_iterador(it, acc));
                        let feito = t!(self.obter(res, ChaveProp::Nome(fixos::DONE)));
                        let v = t!(self.obter(res, ChaveProp::Nome(fixos::VALUE)));
                        if self.verdadeiro(feito) {
                            acc = v;
                            pc = a;
                        } else {
                            let id = self.quadros[qi].suspenso;
                            let salvo = self.salvar_quadro(ini as u32);
                            self.suspensos[id as usize] = Some(salvo);
                            return Ok(Fim::Suspenso(v));
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

    fn chave_privada(&mut self, c: Chave) -> Chave {
        if let Some(&k) = self.privados.get(&c.0) {
            return k;
        }
        let d = self.texto_do_nome(c);
        let s = self.novo_simbolo(d);
        let k = self.chave_do_simbolo(s);
        self.escondidos.insert(k.0 & !Chave::BIT_SIMBOLO);
        self.privados.insert(c.0, k);
        k
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

    fn super_chamar(&mut self, qi: usize, args: &[Valor]) -> Res<Valor> {
        let q = self.quadros[qi];
        // Em seta dentro do construtor, a classe é a da casa (shortcut: só o quadro direto).
        let f = q.funcao;
        let pai = self.proto_de(f);
        if !self.e_construtor(pai) {
            return self.erro_tipo("Super constructor is not a constructor");
        }
        if q.this != Valor::BURACO {
            return self.erro_referencia("Super constructor may only be called once");
        }
        let nt = if q.new_target == Valor::UNDEFINED { f } else { q.new_target };
        let o = self.construir(pai, args, nt)?;
        self.quadros[qi].this = o;
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

    fn registrar_campo(&mut self, classe: Valor, k: Valor, init: Valor) {
        let lista = match self.propria(classe, simb::CAMPOS) {
            Some(d) => self.ler_campo(classe, d.slot),
            None => Valor::LISTA_VAZIA,
        };
        let n = if lista == Valor::LISTA_VAZIA { 0 } else { self.lista_len(lista) };
        let nova = self.redimensionar_lista(lista, n + 2, Valor::UNDEFINED);
        self.lista_set(nova, n, k);
        self.lista_set(nova, n + 1, init);
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
        for (i, &v) in itens.iter().enumerate() {
            self.gravar_elemento(a, i as u32, v);
        }
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

    /// Registro de iterador: lista `[iterador, next, feito]`; array comum: `[array, IT_ARRAY, i]`.
    pub fn obter_iterador(&mut self, v: Valor) -> Res<Valor> {
        let reg = self.nova_lista(3);
        if self.e_array(v) && self.iterador_de_array_intacto(v) {
            self.lista_set(reg, 0, v);
            self.lista_set(reg, 1, IT_ARRAY);
            self.lista_set(reg, 2, Valor::int(0));
            return Ok(reg);
        }
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
        Ok(reg)
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

    /// `next(v)` cru (para `yield*`).
    fn passo_iterador(&mut self, reg: Valor, v: Valor) -> Res<Valor> {
        let it = self.lista_get(reg, 0);
        if self.lista_get(reg, 1) == IT_ARRAY {
            return match self.proximo(reg)? {
                Some(x) => Ok(self.resultado_iterador(x, false)),
                None => Ok(self.resultado_iterador(Valor::UNDEFINED, true)),
            };
        }
        let next = self.lista_get(reg, 1);
        let r = self.chamar(next, it, &[v])?;
        if !self.e_objeto(r) {
            return self.erro_tipo("Iterator result is not an object");
        }
        Ok(r)
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
            for k in self.chaves_de(p, false, true, false) {
                if !vistas.insert(k) {
                    continue;
                }
                let enumeravel = self.descritor_proprio(p, k).is_some_and(|d| d.attrs & attr::ENUMERAVEL != 0);
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

    fn ler_prop_cache(&mut self, proto: usize, ci: u32, o: Valor, c: Chave) -> Res<Valor> {
        if o.e_ref() {
            let fo = self.forma(o).0;
            let cache = self.protos[proto].caches[ci as usize];
            if cache.forma == fo {
                if cache.dono == Valor::UNDEFINED {
                    return Ok(self.ler_campo(o, cache.deslocamento));
                }
                if self.forma(cache.dono).0 == cache.forma_dono {
                    return Ok(self.ler_campo(cache.dono, cache.deslocamento));
                }
            }
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
                let e = &mut self.protos[proto].caches[ci as usize];
                e.forma = fo.0;
                e.dono = dono;
                e.forma_dono = forma_dono;
                e.deslocamento = slot;
            }
        }
        Ok(v)
    }

    fn gravar_prop_cache(&mut self, proto: usize, ci: u32, o: Valor, c: Chave, v: Valor, estrito: bool) -> Res<()> {
        if o.e_ref() {
            let fo = self.forma(o).0;
            let cache = self.protos[proto].caches[ci as usize];
            if cache.forma == fo && cache.dono == Valor::UNDEFINED {
                self.gravar_campo(o, cache.deslocamento, v);
                return Ok(());
            }
        }
        self.gravar(o, ChaveProp::Nome(c), v, estrito)?;
        if self.e_objeto(o) {
            let fo = self.forma(o);
            if !self.formas.get(fo).dicionario && !(c == fixos::LENGTH && self.e_array(o)) {
                if let Some(d) = self.propria(o, c) {
                    if d.attrs & (attr::ACESSOR | attr::GRAVAVEL) == attr::GRAVAVEL {
                        let e = &mut self.protos[proto].caches[ci as usize];
                        e.forma = fo.0;
                        e.dono = Valor::UNDEFINED;
                        e.deslocamento = d.slot;
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
