//! Conjunto de instruções (`ARQUITETURA.md`, seção 6): máquina de registradores com
//! acumulador. Cada instrução ocupa 1 byte de código seguido dos operandos.
//!
//! - `R` registrador, `K` constante, `C` cache, `N` contagem: 1 byte; depois do prefixo
//!   `Largo` são 2 bytes, depois de `Largo4`, 4 bytes.
//! - `I` (imediato com sinal) e `A` (endereço de salto, absoluto no código): sempre 4 bytes.
//!
//! Os nomes são deste projeto. A tabela [`Op::operandos`] é a única descrição do formato:
//! o emissor, o desmontador, o intérprete e o verificador leem dela.

/// Tipo de um operando.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operando {
    R,
    K,
    C,
    N,
    I,
    A,
}

macro_rules! instrucoes {
    ($($(#[$doc:meta])* $nome:ident ( $($op:ident),* )),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
        #[repr(u8)]
        pub enum Op { $($(#[$doc])* $nome),* }

        const OPERANDOS: &[&[Operando]] = &[$(&[$(Operando::$op),*]),*];
        const NOMES: &[&str] = &[$(stringify!($nome)),*];
        pub const TOTAL: usize = NOMES.len();
    };
}

instrucoes! {
    /// Prefixo: os operandos R/K/C/N da próxima instrução têm 2 bytes.
    Largo(),
    /// Prefixo: os operandos R/K/C/N da próxima instrução têm 4 bytes.
    Largo4(),

    // ---- Carregar no acumulador ----
    CarregaUndef(),
    CarregaNulo(),
    CarregaVerdadeiro(),
    CarregaFalso(),
    CarregaBuraco(),
    /// acc = inteiro (cabe em 31 bits).
    CarregaInt(I),
    /// acc = constante k (número, texto, BigInt, modelo).
    CarregaConst(K),
    /// acc = r
    Carrega(R),
    /// r = acc
    Guarda(R),
    /// r0 = r1
    Move(R, R),
    /// acc = this (da função ou seta)
    CarregaThis(),
    CarregaNewTarget(),
    /// acc = função que está rodando
    CarregaFuncao(),

    // ---- Nomes ----
    /// acc = global k (ReferenceError se não existe); cache c.
    LeGlobal(K, C),
    /// acc = global k, ou undefined (para `typeof x`).
    LeGlobalTypeof(K, C),
    /// global k = acc (no estrito, ReferenceError se não existe); cache c.
    GravaGlobal(K, C),
    /// Declara `var`/`function` global k (valor em acc, se N=1).
    DeclaraGlobal(K, N),
    /// Inicia o global k com acc: o léxico (`let`/`const`/`class`), se existe; senão grava a
    /// propriedade do objeto global (`var x = 1`).
    IniciaGlobal(K),
    /// Declara `let`/`const`/`class` global k (N: 0 let, 1 const), começa em TDZ.
    DeclaraGlobalLexico(K, N),
    /// acc = contexto subindo N níveis, campo K.
    LeContexto(N, K),
    /// Como `LeContexto`, mas ReferenceError se ainda está em TDZ.
    LeContextoChecado(N, K),
    GravaContexto(N, K),
    /// Grava em `const` (contexto): TypeError.
    GravaConst(K),
    /// Novo contexto com N campos (todos undefined), filho do atual; vira o atual.
    EmpurraContexto(N),
    /// Volta ao contexto pai.
    SaiContexto(),
    /// Empurra o objeto acc como escopo de `with` (busca dinâmica passa por ele).
    EmpurraCom(),
    /// Troca o contexto atual por uma cópia (nova volta de `for (let …)` com captura).
    RenovaContexto(),
    /// Busca por nome (with/eval), cache c.
    LeDinamico(K, C),
    LeDinamicoTypeof(K, C),
    GravaDinamico(K, C),
    /// acc = algum objeto de `with` nos N primeiros contextos tem o nome K (sem `unscopables`).
    TemCom(K, N),
    /// ReferenceError se acc é o buraco (TDZ) — nome K para a mensagem.
    ChecaTdz(K),

    // ---- Propriedades ----
    /// acc = r.k
    LeProp(R, K, C),
    /// acc = acc.k
    LePropAcc(K, C),
    /// r.k = acc
    GravaProp(R, K, C),
    /// acc = r[acc]
    LeItem(R, C),
    /// r0[r1] = acc
    GravaItem(R, R, C),
    /// acc = delete r.k / delete r[acc]
    ApagaProp(R, K),
    ApagaItem(R),
    /// acc = super.k (this em r)
    LeSuper(R, K),
    GravaSuper(R, K),
    LeSuperItem(R, R),
    GravaSuperItem(R, R, R),
    /// acc = r0.#x, com o nome privado (símbolo escondido) em r1. Só o próprio objeto: sem
    /// protótipo e sem armadilha de Proxy.
    LePrivado(R, R),
    /// r0.#x = acc (nome em r1).
    GravaPrivado(R, R),
    /// acc = #x in acc (nome em r).
    TemPrivado(R),
    /// Acrescenta #x (nome em r1) a r0 com o valor de acc. N: 0 campo, 1 método, 2 getter,
    /// 3 setter. Campo ou método repetido é TypeError.
    DefinePrivado(R, R, N),
    /// acc = nome privado novo (símbolo escondido da reflexão) com a descrição K (`#x`).
    NovoPrivado(K),

    // ---- Literais ----
    /// acc = {} com N campos na linha
    CriaObjeto(N),
    /// acc = [] com capacidade N
    CriaArray(N),
    /// Define r.k = acc (própria, enumerável), sem chamar setter.
    DefProp(R, K, C),
    /// Define r[r1] = acc (própria).
    DefItem(R, R),
    /// Acrescenta acc no fim do array r.
    Empurra(R),
    /// Acrescenta um buraco no fim do array r.
    EmpurraBuraco(R),
    /// Espalha o iterável acc no fim do array r.
    EspalhaArray(R),
    /// acc = cópia das próprias enumeráveis de r0 menos as N chaves em r1… (`{a, ...resto}`).
    RestoObjeto(R, R, N),
    /// Copia as próprias enumeráveis de acc para r (`{...x}`).
    EspalhaObjeto(R),
    /// Define getter (N=0) ou setter (N=1) acc em r[r1]; N|2 = não enumerável (classe).
    DefAcessor(R, R, N),
    /// Define método acc em r[r1], não enumerável (classe).
    DefMetodo(R, R),
    /// r.__proto__ = acc (literal de objeto)
    DefProto(R),
    CriaRegex(K, K),
    /// acc = objeto do template marcado (cozidos e crus em K, cache por local).
    ObjetoTemplate(K),
    /// acc = função nova a partir do protótipo de função K (fecha o contexto atual).
    CriaFuncao(K),
    /// acc = classe: construtor K, herança em r (ou buraco).
    CriaClasse(K, R),
    /// Registra na classe r0 um elemento de instância de chave r1, com o valor em acc. N: 0
    /// campo (acc = inicializador ou undefined), 1/2/3 método/getter/setter privado. O
    /// construtor põe os métodos privados e depois roda os campos, na ordem.
    DefCampo(R, R, N),
    /// Casa (home object) do método em acc = r.
    DefineCasa(R),
    /// SetFunctionName: a função em acc ganha o nome da chave em r (símbolo: `[descrição]`;
    /// nome privado: `#x`). N: 0 sem prefixo, 1 `get `, 2 `set `.
    NomeiaFuncao(R, N),

    // ---- Operadores (r op acc) ----
    Soma(R, C),
    Sub(R, C),
    Mul(R, C),
    Div(R, C),
    Resto(R, C),
    Potencia(R, C),
    E(R, C),
    Ou(R, C),
    OuExclusivo(R, C),
    DeslEsq(R, C),
    DeslDir(R, C),
    DeslDirSemSinal(R, C),
    Igual(R, C),
    Diferente(R, C),
    Identico(R),
    NaoIdentico(R),
    Menor(R, C),
    Maior(R, C),
    MenorIgual(R, C),
    MaiorIgual(R, C),
    /// acc = r in acc
    In(R),
    /// acc = r instanceof acc
    Instanceof(R, C),
    /// Somas com inteiro pequeno no acc (`i + 1`, `x - 2`).
    SomaInt(I, C),
    // ---- Unários (no acc) ----
    Negativo(C),
    ParaNumero(),
    ParaNumerico(),
    Nao(),
    Til(C),
    Typeof(),
    Incrementa(C),
    Decrementa(C),
    /// r = ToNumeric(r) ± 1, sem tocar o acc (`i++` como comando, `i` local em registrador).
    IncrementaReg(R, C),
    DecrementaReg(R, C),
    /// acc = ToInt32(acc) (`x | 0`).
    ParaInt32(C),
    /// acc = String(acc) (templates)
    ParaTexto(),
    /// acc = ToPropertyKey(acc)
    ParaChave(),
    /// acc = ToObject(acc) (TypeError em null/undefined)
    ParaObjeto(),

    // ---- Saltos ----
    Salta(A),
    /// Salto para trás: volta de laço (conta calor, ponto seguro).
    SaltaAtras(A),
    SaltaSeVerdadeiro(A),
    SaltaSeFalso(A),
    SaltaSeNuloOuIndef(A),
    SaltaSeNaoNuloOuIndef(A),
    SaltaSeIndef(A),
    SaltaSeNaoIndef(A),
    /// Pula se acc não é o buraco.
    SaltaSeNaoBuraco(A),

    // ---- Chamadas ----
    /// acc = r0(r1 = this, r1+1 … r1+N) ; cache c
    Chama(R, R, N, C),
    /// Como `Chama`, mas o último argumento é um array a espalhar.
    ChamaEspalhado(R, R, N),
    /// eval direto: como `Chama`, com o escopo atual.
    ChamaEval(R, R, N),
    /// acc = new r0(r1 … r1+N-1) ; new.target = r0
    New(R, R, N, C),
    NewEspalhado(R, R, N),
    /// super(r1 … r1+N-1)
    SuperChama(R, N),
    SuperChamaEspalhado(R, N),
    Retorna(),
    Lanca(),
    /// Relança a exceção guardada em r (fim de finally).
    Relanca(R),
    /// TypeError com a mensagem K.
    LancaErro(K),

    // ---- Iteração ----
    /// r = iterador de acc (Symbol.iterator); N=1: assíncrono.
    ObtemIterador(R, N),
    /// acc = próximo valor de r; salta para A quando acabou.
    Proximo(R, A),
    /// Fecha o iterador r (chama `return`), sem trocar o acc.
    FechaIterador(R),
    /// Fecha o iterador r saindo por exceção: erro de `return` é engolido (o original vence).
    FechaIteradorEmErro(R),
    /// r = estado de for-in sobre acc.
    PreparaForIn(R),
    /// acc = próxima chave do for-in r; salta para A quando acabou.
    ProximoForIn(R, A),

    // ---- Funções ----
    /// acc = objeto arguments (N=1: mapeado aos parâmetros).
    CriaArguments(N),
    /// acc = array com os argumentos a partir do N-ésimo (resto).
    CriaResto(N),
    /// `debugger`.
    Depurador(),

    // ---- Geradores e async ----
    /// Suspende devolvendo acc (yield); ao voltar, acc = valor enviado.
    Suspende(),
    /// Aguarda a promessa acc (await).
    Aguarda(),
    /// Início de corpo de gerador: cria o objeto e suspende.
    IniciaGerador(),
    /// Resultado `{value: acc, done: N}`.
    CriaResultado(N),
    /// Logo depois de `Suspende`: retomado com `return` → salta para A (acc = valor).
    ModoRetomada(A),
    /// `yield*` sobre o registro r: o de dentro acabou → acc = valor, salta para o 1º A;
    /// retomado com `return` e ele acabou (ou não tem `return`) → acc = valor, salta para o 2º A.
    Delega(R, A, A),
    /// `for await`: acc = `next()` do registro r, sem conferir (o resultado é aguardado).
    ChamaProximo(R),
    /// `for await`: confere o resultado aguardado em acc; acabou → salta para A; senão acc = value.
    ResultadoProximo(R, A),
    /// acc = `return()` do iterador r (fechamento assíncrono); sem `return` ou já fechado → A.
    FechaIteradorAsync(R, A),
    /// TypeError se acc não é objeto (resultado do `return()` aguardado).
    ExigeObjeto(),

    // ---- Módulos ----
    ImportDinamico(R),
    ImportMeta(),
    /// acc = valor ao vivo da N-ésima importação (com TDZ).
    LeImport(N),
    /// Valor de `export default <expressão>` = acc.
    GuardaPadrao(),
    /// O contexto atual é o do topo do módulo (de onde as exportações são lidas).
    RegistraModulo(),
    /// acc = namespace do N-ésimo módulo pedido (`import * as`).
    CarregaNamespace(N),

    /// Fim do código (nunca executado; marca o fim para o verificador).
    Fim(),
}

impl Op {
    #[inline]
    pub fn operandos(self) -> &'static [Operando] {
        OPERANDOS[self as usize]
    }

    pub fn nome(self) -> &'static str {
        NOMES[self as usize]
    }

    #[inline]
    pub fn de_byte(b: u8) -> Option<Op> {
        if (b as usize) < TOTAL {
            // SAFETY: `Op` é `repr(u8)` com discriminantes 0..TOTAL, sem buracos.
            Some(unsafe { std::mem::transmute::<u8, Op>(b) })
        } else {
            None
        }
    }

    /// Instrução que encerra o caminho (o seguinte só é alcançado por salto).
    pub fn termina(self) -> bool {
        matches!(self, Op::Salta | Op::SaltaAtras | Op::Retorna | Op::Lanca | Op::Relanca | Op::LancaErro | Op::Fim)
    }
}

/// Lê um operando de largura `escala` (1, 2 ou 4) para R/K/C/N; I e A sempre 4.
#[inline]
pub fn ler_operando(codigo: &[u8], pos: usize, tipo: Operando, escala: usize) -> (u32, usize) {
    let largura = match tipo {
        Operando::I | Operando::A => 4,
        _ => escala,
    };
    let v = match largura {
        1 => u32::from(codigo[pos]),
        2 => u32::from(u16::from_le_bytes([codigo[pos], codigo[pos + 1]])),
        _ => u32::from_le_bytes([codigo[pos], codigo[pos + 1], codigo[pos + 2], codigo[pos + 3]]),
    };
    (v, largura)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn tabela_consistente() {
        assert!(TOTAL < 256);
        for b in 0..TOTAL as u8 {
            let op = Op::de_byte(b).unwrap();
            assert_eq!(op as u8, b);
        }
        assert_eq!(Op::LeProp.operandos(), &[Operando::R, Operando::K, Operando::C]);
        assert!(Op::de_byte(TOTAL as u8).is_none());
    }
}
