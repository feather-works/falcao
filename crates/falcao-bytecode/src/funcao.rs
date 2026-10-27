//! O que o gerador produz: um [`Script`] com as funções em bytecode. Tudo é imutável e não
//! depende do runtime (nomes vão como textos do script; o runtime traduz ao carregar).

/// Constante de uma função (operando `K`).
#[derive(Clone, Debug, PartialEq)]
pub enum Constante {
    Numero(f64),
    /// Texto (índice em [`Script::textos`]) usado como valor.
    Texto(u32),
    /// Texto usado como nome de propriedade ou de variável global.
    Nome(u32),
    /// BigInt pelo texto decimal (índice em `textos`).
    BigInt(u32),
    /// Função interna (índice em [`Script::funcoes`]).
    Funcao(u32),
    /// Template marcado: cozidos (`None` = escape inválido) e crus.
    Template { cozidos: Vec<Option<u32>>, crus: Vec<u32> },
    /// Expressão regular: corpo e flags (índices em `textos`).
    Regex { corpo: u32, flags: u32 },
    /// Classe: construtor (índice da função) e se é derivada.
    Classe { construtor: u32, derivada: bool, nome: Option<u32> },
}

/// Faixa protegida por `try`: se uma exceção acontece em `[inicio, fim)`, o intérprete
/// restaura a profundidade de contexto, guarda a exceção no acc e salta para `alvo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Excecao {
    pub inicio: u32,
    pub fim: u32,
    pub alvo: u32,
    /// Quantos contextos estavam empurrados (dentro da função) na entrada do `try`.
    pub contextos: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub estrita: bool,
    pub seta: bool,
    pub metodo: bool,
    pub construtor_classe: bool,
    pub classe_derivada: bool,
    pub gerador: bool,
    pub assincrona: bool,
    pub getter: bool,
    pub setter: bool,
    /// Não pode ser chamada com `new` (seta, método, gerador, async).
    pub sem_construtor: bool,
    /// Precisa do objeto `arguments`.
    pub usa_arguments: bool,
    pub eval_direto: bool,
    /// O topo do script/módulo.
    pub topo: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FuncaoBytecode {
    /// Nome (índice em `textos`), para `name` e mensagens.
    pub nome: Option<u32>,
    pub codigo: Vec<u8>,
    pub constantes: Vec<Constante>,
    /// Registradores do quadro (os parâmetros são os primeiros).
    pub registradores: u32,
    /// Parâmetros formais (os que ganham registrador).
    pub parametros: u32,
    /// `length` da função (parâmetros antes do primeiro com valor padrão ou resto).
    pub comprimento: u32,
    pub caches: u32,
    pub excecoes: Vec<Excecao>,
    /// (deslocamento no código, byte no fonte), em ordem.
    pub posicoes: Vec<(u32, u32)>,
    pub flags: Flags,
    /// Faixa no fonte (para `toString`).
    pub inicio: u32,
    pub fim: u32,
}

impl FuncaoBytecode {
    /// Posição no fonte da instrução em `pc`.
    pub fn posicao_de(&self, pc: u32) -> u32 {
        match self.posicoes.binary_search_by_key(&pc, |p| p.0) {
            Ok(i) => self.posicoes[i].1,
            Err(0) => self.inicio,
            Err(i) => self.posicoes[i - 1].1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    /// A função 0 é o topo.
    pub funcoes: Vec<FuncaoBytecode>,
    /// Textos (UTF-16), sem repetição.
    pub textos: Vec<Vec<u16>>,
    pub modulo: bool,
    /// Importações e exportações (só em módulo).
    pub info_modulo: InfoModulo,
}

/// O que um módulo pede e oferece. Nomes em UTF-16; `req` é índice em `requisitos`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InfoModulo {
    /// Especificadores (`'./a.js'`), sem repetição, na ordem em que aparecem.
    pub requisitos: Vec<Vec<u16>>,
    /// Vínculos importados (`LeImport k` lê o k-ésimo): (req, nome exportado lá).
    pub importacoes: Vec<(u32, Vec<u16>)>,
    pub exportacoes: Vec<(Vec<u16>, Exportacao)>,
    /// `export * from`.
    pub estrelas: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Exportacao {
    /// Slot no contexto do topo do módulo.
    Local(u32),
    /// `export default <expressão>` (guardado por `GuardaPadrao`).
    Padrao,
    /// Repassa o nome de outro módulo (`export { a } from`, ou importado e exportado).
    Indireta(u32, Vec<u16>),
    /// `export * as ns from`.
    Namespace(u32),
}
