//! O estado de um contexto JS: gaiola, formas, nomes, símbolos e os objetos fixos.

use crate::formas::{self, Chave, FormaId, Formas, Tipo};
use crate::gaiola::Gaiola;
use crate::nomes::Nomes;
use crate::valor::Valor;

pub struct Motor {
    pub g: Gaiola,
    pub formas: Formas,
    pub nomes: Nomes,
    /// Objeto de cada símbolo, pelo número (`Chave::simbolo(i)`).
    pub simbolos: Vec<Valor>,
    /// Textos de um caractere Latin-1 (fixos).
    pub caracteres: [Valor; 256],
    // ---- intérprete ----
    /// Registradores e argumentos de todos os quadros (e argumentos de nativas).
    pub pilha: Vec<Valor>,
    pub quadros: Vec<crate::interpretador::Quadro>,
    pub protos: Vec<crate::funcoes::Proto>,
    pub nativas: Vec<crate::funcoes::Nativa>,
    pub intr: crate::intrinsecos::Intrinsecos,
    /// `let`/`const`/`class` do topo dos scripts: nome → (índice em `lexico_valores`, const).
    pub lexico: std::collections::HashMap<u32, (u32, bool)>,
    pub lexico_valores: Vec<Valor>,
    /// Símbolos que a reflexão não mostra (privados e internos).
    pub escondidos: std::collections::HashSet<u32>,
    /// Algum item indexado tem atributos fora do padrão (mora como nome; ver `propriedades`).
    pub indices_especiais: bool,
    /// Função nativa sendo chamada agora (para nativas que leem o próprio objeto).
    pub chamado: Valor,
    /// Geradores e funções async suspensos (índice no campo interno do objeto).
    pub suspensos: Vec<Option<crate::interpretador::QuadroSalvo>>,
    pub suspensos_livres: Vec<u32>,
    pub microtarefas: std::collections::VecDeque<crate::promessas::Tarefa>,
    /// Promessa de cada async que está rodando agora (pelo número do suspenso).
    pub promessas_async: std::collections::HashMap<u32, Valor>,
    /// Objetos de template marcado já criados.
    pub templates: Vec<Valor>,
    /// Muda quando nasce um `let`/`const` global (invalida o cache de globais).
    pub epoca_lexico: u32,
    /// Valores que o embutidor segura entre chamadas (raízes do coletor).
    pub raizes: Vec<Valor>,
    pub ids_nativas: std::collections::HashMap<&'static str, u32>,
    /// `Symbol.for`.
    pub registro_simbolos: std::collections::HashMap<Vec<u16>, Valor>,
    /// Arrays no meio de `join` (para cortar ciclos).
    pub juntando: Vec<u32>,
    /// Tabelas de Map/Set/WeakMap/WeakSet.
    pub tabelas: Vec<crate::builtins::colecoes::Tabela>,
    /// Programas de regex compilados e o cache por (fonte, flags).
    pub regexes: Vec<std::rc::Rc<crate::regex::Programa>>,
    pub cache_regex: std::collections::HashMap<(Vec<u16>, Vec<u16>), u32>,
    /// Formas 2 a 4 dos caches de propriedade polimórficos (`CacheProp::poli`).
    pub(crate) poli: Vec<[crate::funcoes::CacheProp; 3]>,
    /// Caches megamórficos: (forma, nome) → onde está (tabela global da seção 8).
    pub(crate) mega: falcao_sintaxe::atomos::MapaRapido<(u32, Chave), crate::funcoes::CacheProp>,
    /// Profundidade de laços do intérprete abertos por nativas (o coletor só roda no 0).
    pub aninhamento: u32,
    /// Quantos desses níveis guardam em `raizes` tudo o que usam depois da chamada
    /// ([`Motor::chamar_coletavel`]): com todos seguros, o coletor roda lá dentro.
    pub(crate) seguro: u32,
    /// Saída do `console` (nível, texto). Sem ela, vai para a saída padrão.
    pub ao_imprimir: Option<Box<dyn FnMut(u8, &str)>>,
    /// Olhado a cada 65.536 voltas de laço: `Some(motivo)` interrompe o script com um Error
    /// (prazo, botão Parar). Quem embute decide; uma vez interrompido, a olhada continua dizendo sim.
    pub interromper: Option<Box<dyn FnMut() -> Option<String>>>,
    /// A gaiola passou do teto: o erro sobe sem parar em `catch`/`finally` até quem embute
    /// (como a aba que cai no navegador), senão o próprio `catch` estouraria de novo.
    pub(crate) terminando: bool,
    /// Endereço da pilha do Rust na entrada mais externa do JS (0 fora dele).
    pub(crate) base_pilha: usize,
    /// Bytes da pilha do Rust que o motor pode usar abaixo da entrada antes de dar
    /// "Maximum call stack size exceeded" (Proxy de Proxy, `bind` de `bind`, JSON fundo…).
    /// O padrão cabe numa thread de 2 MB; quem embute numa pilha maior pode subir.
    pub limite_pilha: usize,
    /// O último erro que o próprio motor criou e a profundidade da pilha naquela hora: o
    /// intérprete refaz a pilha dele já com a instrução certa ([`Motor::reanotar_erro`]).
    pub erro_interno: (Valor, u32),
    pub voltas: u32,
    /// Dados de quem embute o motor (o navegador guarda a página aqui), lidos pelas nativas dele.
    pub hospede: Option<Box<dyn std::any::Any>>,
    /// Promessas rejeitadas ainda sem tratador (viram o "Uncaught (in promise)" do console).
    pub rejeicoes: Vec<Valor>,
    /// Coletas feitas: [menores, maiores].
    pub coletas: [u32; 2],
    /// Bytes vivos na velha depois da última coleta maior (a próxima vem quando dobrar).
    pub vivo_apos_maior: u64,
    /// Coleta em todo ponto seguro (`FALCAO_GC_ESTRESSE=1`), para achar `Valor` solto no Rust.
    pub estresse: bool,
    /// Módulos ES carregados (o índice é o `Proto::modulo`).
    pub modulos: Vec<crate::modulos::Modulo>,
    pub modulos_por_url: std::collections::HashMap<String, u32>,
    /// Onde o embutidor baixa os módulos (sem ele, `import` de outro arquivo falha).
    pub carregador_modulos: Option<Box<dyn crate::modulos::CarregadorModulos>>,
    /// URL de quem roda os scripts comuns (base do `import()` fora de módulo).
    pub url_base: String,
    /// `eval`, `new Function` e o `eval` direto perguntam aqui antes de compilar o texto
    /// (HostEnsureCanCompileStrings: a CSP do navegador); `Err` vira EvalError.
    pub ao_compilar_texto: Option<fn(&mut Motor) -> Result<(), String>>,
}

/// Identificador do objeto fixo, na palavra 1 dele.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum Fixo {
    Undefined = 0,
    Null = 1,
    True = 2,
    False = 3,
    Buraco = 4,
}

impl Default for Motor {
    fn default() -> Self {
        Self::novo()
    }
}

impl Motor {
    pub fn novo() -> Self {
        let mut m = Motor {
            g: Gaiola::nova(),
            formas: Formas::nova(),
            nomes: Nomes::novo(),
            simbolos: Vec::new(),
            caracteres: [Valor::UNDEFINED; 256],
            pilha: Vec::with_capacity(1 << 16),
            quadros: Vec::with_capacity(256),
            protos: Vec::new(),
            nativas: Vec::new(),
            intr: crate::intrinsecos::Intrinsecos::vazios(),
            lexico: Default::default(),
            lexico_valores: Vec::new(),
            escondidos: Default::default(),
            indices_especiais: false,
            chamado: Valor::UNDEFINED,
            suspensos: Vec::new(),
            suspensos_livres: Vec::new(),
            microtarefas: Default::default(),
            promessas_async: Default::default(),
            templates: Vec::new(),
            epoca_lexico: 0,
            raizes: Vec::new(),
            ids_nativas: Default::default(),
            registro_simbolos: Default::default(),
            juntando: Vec::new(),
            tabelas: Vec::new(),
            regexes: Vec::new(),
            cache_regex: Default::default(),
            poli: Vec::new(),
            mega: Default::default(),
            aninhamento: 0,
            seguro: 0,
            ao_imprimir: None,
            interromper: None,
            terminando: false,
            base_pilha: 0,
            limite_pilha: 1 << 20,
            erro_interno: (Valor::UNDEFINED, 0),
            voltas: 0,
            hospede: None,
            rejeicoes: Vec::new(),
            coletas: [0; 2],
            vivo_apos_maior: 0,
            estresse: std::env::var_os("FALCAO_GC_ESTRESSE").is_some_and(|v| v == "1"),
            modulos: Vec::new(),
            modulos_por_url: Default::default(),
            carregador_modulos: None,
            url_base: String::new(),
            ao_compilar_texto: None,
        };
        // Os fixos nascem na ordem das constantes de `Valor`.
        for (i, esperado) in [Valor::UNDEFINED, Valor::NULL, Valor::TRUE, Valor::FALSE, Valor::BURACO].into_iter().enumerate() {
            let d = m.g.alocar_permanente(8);
            assert_eq!(Valor::de_deslocamento(d), esperado);
            m.g.por_u32(d, formas::FIXO.0);
            m.g.por_u32(d + 4, i as u32);
        }
        let d = m.g.alocar_permanente(16);
        assert_eq!(Valor::de_deslocamento(d), Valor::VAZIA);
        m.g.por_u32(d, formas::LATIN1.0);
        let d = m.g.alocar_permanente(8);
        assert_eq!(Valor::de_deslocamento(d), Valor::LISTA_VAZIA);
        m.g.por_u32(d, formas::LISTA.0);
        for c in 0..256u32 {
            let d = m.g.alocar_permanente(17);
            m.g.por_u32(d, formas::LATIN1.0);
            m.g.por_u32(d + 4, 1);
            m.g.bytes_mut(d + 16, 1)[0] = c as u8;
            m.caracteres[c as usize] = Valor::de_deslocamento(d);
        }
        m.iniciar_intrinsecos();
        crate::builtins::instalar(&mut m);
        m.g.coleta_pendente |= m.estresse;
        m
    }

    // ---------------------------------------------------------------- tipos

    #[inline(always)]
    pub fn forma(&self, v: Valor) -> FormaId {
        FormaId(self.g.forma_de(v))
    }

    #[inline(always)]
    pub fn tipo(&self, v: Valor) -> Option<Tipo> {
        v.e_ref().then(|| self.formas.get(self.forma(v)).tipo)
    }

    #[inline]
    pub fn e_texto(&self, v: Valor) -> bool {
        v.e_ref() && matches!(self.forma(v), formas::LATIN1 | formas::UTF16 | formas::CORDA)
    }

    #[inline]
    pub fn e_numero(&self, v: Valor) -> bool {
        v.e_int() || self.forma(v) == formas::NUMERO
    }

    #[inline]
    pub fn e_objeto(&self, v: Valor) -> bool {
        v.e_ref() && self.formas.get(self.forma(v)).tipo == Tipo::Objeto
    }

    #[inline]
    pub fn e_simbolo(&self, v: Valor) -> bool {
        v.e_ref() && self.forma(v) == formas::SIMBOLO
    }

    #[inline]
    pub fn e_bigint(&self, v: Valor) -> bool {
        v.e_ref() && self.forma(v) == formas::BIGINT
    }

    // ---------------------------------------------------------------- números

    /// Número como valor: inteiro de 31 bits direto (menos o `-0`), senão em caixa.
    #[inline]
    pub fn numero(&mut self, x: f64) -> Valor {
        let i = x as i32;
        if i as f64 == x && (i != 0 || x.is_sign_positive()) && (crate::valor::INT_MIN..=crate::valor::INT_MAX).contains(&i) {
            return Valor::int(i);
        }
        self.caixa_numero(x)
    }

    pub fn caixa_numero(&mut self, x: f64) -> Valor {
        let d = self.g.alocar(16);
        self.g.por_u32(d, formas::NUMERO.0);
        self.g.por_f64(d + 8, x);
        Valor::de_deslocamento(d)
    }

    /// O `f64` de um valor que é número (inteiro ou caixa).
    #[inline]
    pub fn f64_de(&self, v: Valor) -> f64 {
        if v.e_int() {
            v.como_int() as f64
        } else {
            debug_assert_eq!(self.forma(v), formas::NUMERO);
            self.g.f64_em(v.deslocamento() + 8)
        }
    }

    // ---------------------------------------------------------------- listas

    /// Lista de `n` valores, todos `UNDEFINED`.
    pub fn nova_lista(&mut self, n: u32) -> Valor {
        if n == 0 {
            return Valor::LISTA_VAZIA;
        }
        let d = self.g.alocar(8 + 4 * n);
        self.g.por_u32(d, formas::LISTA.0);
        self.g.por_u32(d + 4, n);
        for i in 0..n {
            self.g.por_u32(d + 8 + 4 * i, Valor::UNDEFINED.0);
        }
        Valor::de_deslocamento(d)
    }

    #[inline]
    pub fn lista_len(&self, l: Valor) -> u32 {
        self.g.u32_em(l.deslocamento() + 4)
    }

    #[inline]
    pub fn lista_get(&self, l: Valor, i: u32) -> Valor {
        debug_assert!(i < self.lista_len(l));
        self.g.valor_em(l.deslocamento() + 8 + 4 * i)
    }

    #[inline]
    pub fn lista_set(&mut self, l: Valor, i: u32, v: Valor) {
        debug_assert!(i < self.lista_len(l));
        let d = l.deslocamento();
        self.g.por_valor(d, d + 8 + 4 * i, v);
    }

    /// Cópia da lista com `n` itens (corta ou completa com `preencher`).
    pub fn redimensionar_lista(&mut self, l: Valor, n: u32, preencher: Valor) -> Valor {
        let velho = if l == Valor::LISTA_VAZIA { 0 } else { self.lista_len(l) };
        let nova = self.nova_lista(n);
        if n == 0 {
            return nova;
        }
        let k = velho.min(n);
        if k > 0 {
            self.g.copiar(l.deslocamento() + 8, nova.deslocamento() + 8, 4 * k);
            if !self.g.no_bercario(nova.deslocamento()) {
                // Cópia em bloco para a velha: passa os itens pela barreira.
                for i in 0..k {
                    let v = self.lista_get(nova, i);
                    self.lista_set(nova, i, v);
                }
            }
        }
        for i in k..n {
            self.lista_set(nova, i, preencher);
        }
        nova
    }

    // ---------------------------------------------------------------- símbolos

    /// Símbolo novo com a descrição dada (`UNDEFINED` = sem descrição).
    pub fn novo_simbolo(&mut self, descricao: Valor) -> Valor {
        let id = self.simbolos.len() as u32;
        let d = self.g.alocar_velho(12);
        self.g.por_u32(d, formas::SIMBOLO.0);
        self.g.por_u32(d + 4, id);
        self.g.por_valor(d, d + 8, descricao);
        let v = Valor::de_deslocamento(d);
        self.simbolos.push(v);
        v
    }

    pub fn chave_do_simbolo(&self, s: Valor) -> Chave {
        Chave::simbolo(self.g.u32_em(s.deslocamento() + 4))
    }

    pub fn descricao_do_simbolo(&self, s: Valor) -> Valor {
        self.g.valor_em(s.deslocamento() + 8)
    }

    /// Hash estável de uma chave (texto do nome; símbolos pelo número).
    pub fn hash_chave(&self, c: Chave) -> u64 {
        if c.e_simbolo() {
            u64::from(c.0) ^ 0x5bd1_e995
        } else {
            self.nomes.hash_estavel(c)
        }
    }
}
