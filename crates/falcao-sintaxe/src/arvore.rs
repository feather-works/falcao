//! A árvore de um script, numa arena: um vetor de nós; um filho é um índice de 32 bits
//! ([`NoId`]), uma lista é uma faixa num vetor de índices ([`Lista`]). Jogar a árvore fora é
//! liberar três vetores, sem andar nó por nó (`ARQUITETURA.md`, seção 4).

use crate::atomos::Atomo;

/// Índice de um nó na arena. [`NoId::NADA`] faz o papel de `Option::None`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NoId(pub u32);

impl NoId {
    pub const NADA: NoId = NoId(u32::MAX);

    #[inline]
    pub fn existe(self) -> bool {
        self.0 != u32::MAX
    }

    #[inline]
    pub fn opcao(self) -> Option<NoId> {
        self.existe().then_some(self)
    }
}

/// Faixa de [`Arvore::listas`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Lista {
    pub inicio: u32,
    pub len: u32,
}

/// Índice em [`Arvore::funcoes`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FuncaoId(pub u32);

/// Índice em [`Arvore::classes`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ClasseId(pub u32);

/// Texto literal: faixa do fonte (sem escapes) ou índice na tabela do léxico (UTF-16).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextoRef {
    Faixa(u32, u32),
    Tabela(u32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpUnario {
    Menos, Mais, Nao, Til, Typeof, Void, Delete,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpBinario {
    Soma, Sub, Mul, Div, Resto, Potencia,
    DeslEsq, DeslDir, DeslDirSemSinal, E, Ou, OuExclusivo,
    Igual, Diferente, Identico, NaoIdentico, Menor, Maior, MenorIgual, MaiorIgual,
    In, Instanceof,
    /// `#x in obj`.
    InPrivado,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpLogico {
    E, Ou, Coalesce,
}

/// `=`, `+=`... e os lógicos `&&=`, `||=`, `??=`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpAtrib {
    Simples,
    Bin(OpBinario),
    Log(OpLogico),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoVar {
    Var, Let, Const, Using, AwaitUsing,
}

/// Chave de propriedade (objeto literal, classe, padrão de objeto).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Chave {
    Atomo(Atomo),
    Texto(TextoRef),
    Numero(f64),
    BigInt(u32),
    /// `[expressão]`.
    Calculada(NoId),
    /// `#nome` (só em classe).
    Privada(Atomo),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoProp {
    /// `chave: valor`.
    Normal,
    /// `{ x }` (o valor é `Nome(x)`); em padrão pode ter padrão `{ x = 1 }`.
    Curta,
    Metodo,
    Get,
    Set,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum No {
    // ---- Expressões primárias ----
    Nome(Atomo),
    This,
    Super,
    Nulo,
    Verdadeiro,
    Falso,
    Numero(f64),
    /// Índice do texto decimal na tabela do léxico.
    BigInt(u32),
    Texto(TextoRef),
    /// Índice do pedaço em `Lexico::template` (cozido e cru).
    PedacoTemplate(u32),
    /// `` `a${x}b` ``: `pedacos` tem um a mais que `exprs`.
    Template { pedacos: Lista, exprs: Lista },
    /// `` tag`...` `` (`quasi` é um `Template`).
    TemplateMarcado { tag: NoId, quasi: NoId },
    Regex { corpo: (u32, u32), flags: (u32, u32) },
    /// Itens de array (com `Buraco` e `Espalhar`).
    Array(Lista),
    /// Lista de `Propriedade` e `Espalhar`.
    Objeto(Lista),
    Propriedade { chave: Chave, valor: NoId, tipo: TipoProp },
    Funcao(FuncaoId),
    Classe(ClasseId),
    Buraco,
    Espalhar(NoId),

    // ---- Membro, chamada ----
    Membro { obj: NoId, prop: Atomo, opcional: bool },
    Indice { obj: NoId, prop: NoId, opcional: bool },
    MembroPrivado { obj: NoId, prop: Atomo, opcional: bool },
    /// `#x` sozinho, no lado esquerdo de `#x in obj`.
    NomePrivado(Atomo),
    Chamada { func: NoId, args: Lista, opcional: bool },
    New { func: NoId, args: Lista },
    /// Fronteira de uma cadeia com `?.`: se algum `?.` dentro der `null`/`undefined`, tudo é `undefined`.
    CadeiaOpcional(NoId),
    NewTarget,
    ImportMeta,
    ImportDinamico { arg: NoId, opcoes: NoId },

    // ---- Operadores ----
    Unario { op: OpUnario, arg: NoId },
    Atualizacao { incremento: bool, prefixo: bool, alvo: NoId },
    Binario { op: OpBinario, esq: NoId, dir: NoId },
    Logico { op: OpLogico, esq: NoId, dir: NoId },
    Condicional { teste: NoId, sim: NoId, nao: NoId },
    Atribuicao { op: OpAtrib, alvo: NoId, valor: NoId },
    Sequencia(Lista),
    Yield { arg: NoId, delegado: bool },
    Await(NoId),
    /// `(expr)`: fica na árvore para a sintaxe distinguir `(a) = 1` de `({a}) = 1`.
    Paren(NoId),

    // ---- Padrões (lado esquerdo de atribuição, parâmetros, declarações) ----
    PadraoArray(Lista),
    /// Lista de `Propriedade` (o valor é o padrão) e `PadraoResto`.
    PadraoObjeto(Lista),
    /// `alvo = padrao` (valor padrão).
    PadraoComValor { alvo: NoId, valor: NoId },
    PadraoResto(NoId),

    // ---- Comandos ----
    Bloco(Lista),
    Vazio,
    Expressao(NoId),
    /// Lista de `Declarador`.
    Var { tipo: TipoVar, decls: Lista },
    Declarador { alvo: NoId, valor: NoId },
    Se { teste: NoId, sim: NoId, nao: NoId },
    Para { inicio: NoId, teste: NoId, passo: NoId, corpo: NoId },
    ParaEm { esq: NoId, dir: NoId, corpo: NoId },
    ParaDe { esq: NoId, dir: NoId, corpo: NoId, aguarda: bool },
    Enquanto { teste: NoId, corpo: NoId },
    FacaEnquanto { corpo: NoId, teste: NoId },
    Continue(Option<Atomo>),
    Break(Option<Atomo>),
    Retorne(NoId),
    Com { obj: NoId, corpo: NoId },
    /// Lista de `Caso`.
    Escolha { disc: NoId, casos: Lista },
    /// `teste` NADA = `default`.
    Caso { teste: NoId, corpo: Lista },
    Rotulo { rotulo: Atomo, corpo: NoId },
    Lance(NoId),
    Tente { bloco: NoId, param: NoId, captura: NoId, finalmente: NoId },
    Depurador,
    DeclFuncao(FuncaoId),
    DeclClasse(ClasseId),

    // ---- Módulos ----
    /// `import ... from "x"`: lista de `Importado`; `fonte` é um `Texto`.
    Import { itens: Lista, fonte: NoId },
    /// `nome` NADA = importação de namespace (`* as local`); `local` é o nome na casa.
    Importado { nome: Option<Atomo>, nome_texto: NoId, local: Atomo, padrao: bool },
    /// `export { a as b }` (com `fonte` opcional) e `export * from`.
    ExportLista { itens: Lista, fonte: NoId },
    /// `local` de fora → `exportado`.
    Exportado { local: Option<Atomo>, local_texto: NoId, exportado: Option<Atomo>, exportado_texto: NoId },
    ExportTudo { como: Option<Atomo>, como_texto: NoId, fonte: NoId },
    /// `export var/let/const/function/class`.
    ExportDecl(NoId),
    /// `export default expr|function|class`.
    ExportPadrao(NoId),
}

/// Flags de uma função.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FlagsFuncao {
    pub assincrona: bool,
    pub gerador: bool,
    pub seta: bool,
    /// Método de objeto/classe (tem `super` de propriedade, não é construtor).
    pub metodo: bool,
    pub construtor_classe: bool,
    pub classe_derivada: bool,
    pub getter: bool,
    pub setter: bool,
    pub estrita: bool,
    /// Parâmetros só com nomes simples (sem padrão, valor ou resto).
    pub parametros_simples: bool,
    /// Usa `eval(...)` direto no corpo (desliga a resolução estática de nomes nela).
    pub eval_direto: bool,
    pub usa_arguments: bool,
    pub usa_this: bool,
    /// Expressão de função nomeada (`function f(){}` como valor): `f` visível dentro dela.
    pub expressao_nomeada: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Funcao {
    pub nome: Option<Atomo>,
    /// Padrões dos parâmetros.
    pub params: Lista,
    /// Comandos do corpo. Seta com corpo de expressão: um único `Retorne`.
    pub corpo: Lista,
    pub flags: FlagsFuncao,
    /// Faixa no fonte (do início da função ao `}` final), para `toString` e erros.
    pub inicio: u32,
    pub fim: u32,
    /// Função que envolve esta (`None` = topo do script).
    pub pai: Option<FuncaoId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TipoMembroClasse {
    Metodo,
    Get,
    Set,
    Campo,
    /// `accessor x` (decorators): getter + setter com campo privado.
    Acessor,
    /// `static { ... }` (o valor é uma função sem parâmetros).
    BlocoEstatico,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MembroClasse {
    pub chave: Chave,
    pub tipo: TipoMembroClasse,
    pub estatico: bool,
    /// Método/getter/setter: `Funcao`; campo: inicializador (função sem parâmetros) ou NADA.
    pub valor: NoId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Classe {
    pub nome: Option<Atomo>,
    pub heranca: NoId,
    /// O `constructor` (gerado se não houver).
    pub construtor: Option<FuncaoId>,
    pub membros: Vec<MembroClasse>,
    pub inicio: u32,
    pub fim: u32,
}

/// Um script ou módulo inteiro.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arvore {
    pub nos: Vec<No>,
    /// Posição (byte no fonte) onde cada nó começa, paralelo a `nos`.
    pub posicoes: Vec<u32>,
    pub listas: Vec<NoId>,
    pub funcoes: Vec<Funcao>,
    pub classes: Vec<Classe>,
    /// Comandos do topo.
    pub corpo: Lista,
    pub estrito: bool,
    pub modulo: bool,
    /// Módulo com `await` fora de função (o topo vira assíncrono).
    pub await_topo: bool,
}

impl Arvore {
    #[inline]
    pub fn no(&self, id: NoId) -> &No {
        &self.nos[id.0 as usize]
    }

    #[inline]
    pub fn posicao(&self, id: NoId) -> u32 {
        self.posicoes[id.0 as usize]
    }

    #[inline]
    pub fn lista(&self, l: Lista) -> &[NoId] {
        &self.listas[l.inicio as usize..(l.inicio + l.len) as usize]
    }

    pub fn funcao(&self, f: FuncaoId) -> &Funcao {
        &self.funcoes[f.0 as usize]
    }

    pub fn classe(&self, c: ClasseId) -> &Classe {
        &self.classes[c.0 as usize]
    }
}
