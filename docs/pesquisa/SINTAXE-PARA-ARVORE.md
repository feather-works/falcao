# Falcão: inventário de sintaxe para o analisador (texto → tokens → AST)

Fonte normativa: ECMAScript, https://tc39.es/ecma262/ (edição atual, páginas `multipage`).
Seções citadas: 12 (léxico), 13 (expressões), 14 (comandos e declarações), 15 (funções e classes),
16 (scripts e módulos), Anexo B (recursos adicionais para navegadores).

## 0. Estado desta pesquisa (leia antes de usar)

Esta pesquisa não está completa como inventário fechado, e isto precisa ficar claro.

- A página da spec é longa (mais de 500 mil caracteres por seção). A leitura pela ferramenta
  devolveu, em cada trecho, ou o índice da spec, ou um recorte truncado. Por isso não foi
  possível ler o texto normativo inteiro de todas as seções.
- Itens marcados como **[LIDO]** foram conferidos no texto que a ferramenta trouxe.
- Itens marcados como **[CONFERIR]** foram escritos a partir do conhecimento da gramática
  publicada e precisam ser comparados com a spec antes de virar código. Os números de seção
  desses itens são os da spec, mas não foram confirmados por leitura direta.
- Nada aqui veio de código de outro motor. A regra do projeto foi respeitada.

Lista do que foi **[LIDO]** no texto trazido:

- 12.2 Espaço em branco (TAB, VT, FF, ZWNBSP, USP); 12.3 terminadores de linha (LF, CR, LS, PS);
  12.4 comentários (MultiLineComment, SingleLineComment); 12.5 hashbang; 12.6 tokens;
  12.7 nomes (IdentifierName, PrivateIdentifier, IdentifierStart/Part, escapes \u e \u{});
  12.7.1.1 a 12.7.1.3 (erros de escape em identificadores).
- 12.7.2 palavras reservadas e contextuais (lista de categorias).
- 12.8 pontuadores (categorias: OptionalChainingPunctuator, OtherPunctuator, DivPunctuator,
  RightBracePunctuator). A lista completa de OtherPunctuator está no texto da spec, não
  transcrita aqui: ver seção 12.8 e a lista da seção 6 deste documento.
- 12.9.1 a 12.9.6: NullLiteral, BooleanLiteral, NumericLiteral (com BigInt `n` e separador `_`),
  StringLiteral (escapes, LineContinuation, erros de octal legado em strict),
  RegularExpressionLiteral (BodyText, FlagText), Template (TV, TRV, NotEscapeSequence).
- 12.10.1 regras de ASI e lista de restricted productions; 12.10.3.1 casos de início de linha.
- 13.1 identificadores (IdentifierReference, BindingIdentifier, LabelIdentifier, erros de `yield`
  `await` `arguments` `eval`).
- 13.2.4 Array, 13.2.5 Object (PropertyDefinition, PropertyName, ComputedPropertyName,
  CoverInitializedName, erro de `__proto__` duplicado), 13.2.7 regex literal, 13.2.8 template,
  13.2.9 agrupamento.
- 13.3 parcial: MemberExpression, CallExpression, NewExpression, SuperProperty/SuperCall,
  MetaProperty (new.target, import.meta), ImportCall, OptionalExpression/OptionalChain.
  Erros: cadeia opcional com template; import.meta fora de Module.
- 13.15 parcial: AssignmentExpression (condicional, yield, arrow, atribuição simples, composta,
  lógica &&= ||= ??=); erros de alvo válido.
- 14.2.1 (Block, duplicidade lexical e conflito com var), 14.3.1.1 (LexicalDeclaration: `let`
  como nome proibido, `const` exige inicializador, `using`/`await using`), 14.3.2
  VariableStatement, 14.4 EmptyStatement, 14.5 ExpressionStatement, 14.6 if (erro de função
  rotulada na cláusula), 14.7.2 do-while, 14.7.3 while, 14.7.4 for (3 formas de inicialização),
  14.7.5 for-in / for-of / for-await-of (3 formas cada, com erros 9 a 14 da seção 8).

Lista do que **não** foi lido no texto e está **[CONFERIR]**: 13.4 a 13.14 (detalhes de operadores,
exceto a precedência listada na seção 4), 13.15.5 (desestruturação de atribuição), 13.16 (vírgula),
15 inteira (funções, métodos, classes, arrow, async), 16 inteira (import/export),
14.7.5.8 em diante (continue, break, return, with, switch, labelled, throw, try, debugger),
e o Anexo B inteiro.

---

## 1. Léxico (seção 12)

### 1.1 Espaço em branco e terminadores (12.2, 12.3) [LIDO]
- WhiteSpace: TAB (U+0009), VT (U+000B), FF (U+000C), ZWNBSP (U+FEFF), USP (qualquer
  caractere da categoria Unicode Zs).
- LineTerminator: LF (U+000A), CR (U+000D), LS (U+2028), PS (U+2029).
- LineTerminatorSequence: LF | CR [lookahead ≠ LF] | LS | PS | CR LF.
- Nota: LS e PS são permitidos dentro de string (ES2019), mas não são permitidos como LF/CR
  "crus" dentro de string.

### 1.2 Format-control (12.1) [CONFERIR]
- Caracteres de controle de formato (ex.: ZWJ, ZWNJ, direcionais) são tratados como
  ignoráveis fora de identificadores e literais. Conferir a lista exata na 12.1.

### 1.3 Comentários (12.4) [LIDO]
- Comment: MultiLineComment | SingleLineComment.
- MultiLineComment: `/*` ... `*/`. Pode conter quebra de linha; quebra de linha dentro de comentário
  multilinha conta como LineTerminator para ASI.
- SingleLineComment: `//` até o fim da linha.

### 1.4 Comentários HTML-like (Anexo B, B.1.1) [CONFERIR]
- SingleLineHTMLOpenComment: `<!--` em qualquer posição (fora de string e de regex) comenta até o fim da linha.
- SingleLineHTMLCloseComment: `-->` no início de linha (após espaço/comentário) comenta a linha.
- Não vale no goal symbol Module.

### 1.5 Hashbang (12.5) [LIDO]
- HashbangComment: `#!` seguido de caracteres até o fim da linha. Só no início do Script ou Module.

### 1.6 Tokens (12.6) [LIDO]
- CommonToken: IdentifierName | PrivateIdentifier | Punctuator | NumericLiteral | StringLiteral | Template.
- Fora dessa lista: DivPunctuator, RegularExpressionLiteral, RightBracePunctuator e
  TemplateSubstitutionTail, que dependem do contexto (seção 1.11).

### 1.7 Nomes e palavras (12.7) [LIDO]
- IdentifierName: IdentifierStart | IdentifierName IdentifierPart.
- IdentifierStart: IdentifierStartChar | `\` UnicodeEscapeSequence.
- IdentifierStartChar: UnicodeIDStart | `$` | `_`.
- IdentifierPartChar: UnicodeIDContinue | `$`. (ZWNJ e ZWJ aparecem como continuação em identificadores
  só dentro de regra específica; conferir a 12.7.)
- IdentifierPart: IdentifierPartChar | `\` UnicodeEscapeSequence.
- UnicodeEscapeSequence: `\u` Hex4Digits | `\u{` CodePoint `}`.
- PrivateIdentifier: `#` IdentifierName (12.7 e 13.x para uso).
- Early Errors 12.7.1.1: escape Unicode que não produz caractere válido de IdentifierStart/Part é erro.
- Semântica 12.7.1.2 e 12.7.1.3: o nome é comparado pelo valor normalizado (escapes contam para
  igualdade). Ou seja, `var` é `var`, e conta como palavra reservada.

### 1.8 Palavras reservadas e contextuais (12.7.2) [LIDO]
Reservadas sempre (não podem ser identificador):
`await` (só como reservada em módulo e em função async; ver abaixo), `break case catch class const
continue debugger default delete do else enum export extends false finally for function if import
in instanceof new null return super switch this throw true try typeof var void while with`,
e `yield` (reservada dentro de gerador e em strict).

Reservadas em strict mode (ok como nome fora de strict):
`implements interface let package private protected public static yield`.

Contextuais (são identificadores comuns, mas têm sintaxe própria):
`as async from get of set target meta` e `let`, `static`, `await`, `yield` (conforme contexto).

Nota de 12.7.2 [LIDO]: `enum` é reservada futura, sempre proibida.

### 1.9 Pontuadores (12.8) [LIDO em categorias; lista completa abaixo a conferir]
Lista usada (conferir em 12.8 se algo faltar):
- Delimitadores: `{ } ( ) [ ] . ... ; , `
- Relacionais: `< > <= >= == != === !==`
- Aritméticos: `+ - * / % ** ++ --`
- Deslocamento: `<< >> >>>`
- Bit a bit: `& | ^ ~`
- Lógicos: `! && || ??`
- Condicional e opcional: `? : ?.` (`?.` não pode ser seguido de dígito decimal: `a?.5:1` é condicional)
- Atribuição: `= += -= *= /= %= **= <<= >>= >>>= &= |= ^= &&= ||= ??=`
- Seta: `=>`
- Privado: `#` (apenas como início de PrivateIdentifier)
- Categorias da gramática: DivPunctuator (`/`, `/=`), RightBracePunctuator (`}`),
  OptionalChainingPunctuator (`?.` fora de dígito), OtherPunctuator (o resto). [LIDO]

### 1.10 Literais (12.9)

**Null e booleanos (12.9.1, 12.9.2)** [LIDO]: `null`, `true`, `false`.

**Numéricos (12.9.3)** [LIDO em estrutura; detalhes a conferir]
- DecimalLiteral: `0` | `NonZeroDigit` [separadores] | `.` digitos [exp] | digitos `.` [digitos] [exp].
- Expoente: `e` ou `E`, sinal opcional, dígitos.
- Hexadecimal: `0x`/`0X` + dígitos hex.
- Octal moderno: `0o`/`0O` + dígitos 0-7.
- Binário: `0b`/`0B` + dígitos 0-1.
- BigInt: sufixo `n` em inteiro decimal, hex, octal ou binário (não em ponto flutuante nem em legado).
- Separador `_` (NumericLiteralSeparator): permitido entre dígitos; não no começo, no fim,
  junto a `.`, junto a `e`/`x`/`o`/`b` prefixo, nem duplo. [CONFERIR as restrições exatas.]
- Legado (Anexo B, B.1.2) [CONFERIR]: LegacyOctalIntegerLiteral `0` seguido de 0-7 (ex.: `017`),
  e NonOctalDecimalIntegerLiteral `0` seguido de dígitos que contêm 8 ou 9 (ex.: `08`, `09.5`).
  Em strict: erro de sintaxe.
- Regra léxica: o caractere logo depois de um NumericLiteral não pode ser IdentifierStart nem
  DecimalDigit (`3in` é erro). [LIDO]

**Strings (12.9.4)** [LIDO]
- Delimitadas por `'` ou `"`. LF e CR cruos não são permitidos; LS e PS são.
- Escapes:
  - SingleEscapeCharacter: `\b \t \n \v \f \r \" \' \\`.
  - `\0` (quando não seguido de dígito decimal).
  - `\xHH` (dois dígitos hex).
  - `\uHHHH` (quatro dígitos hex).
  - `\u{H...}` (code point até U+10FFFF).
  - LineContinuation: `\` seguido de LineTerminatorSequence (continua a string sem incluir a quebra).
  - Outro caractere após `\` vale ele mesmo (NonEscapeCharacter), ex.: `\a` é `a`.
- Anexo B (B.1.2) [CONFERIR]: octal legado `\1` a `\7` e `\00`..`\377`; `\8` e `\9` (NonOctalDecimalEscapeSequence).
  Em strict: erro de sintaxe, inclusive quando a diretiva "use strict" vem depois da string
  (exemplo de 12.9.4.1 [LIDO]: `function invalid() { "\7"; "use strict"; }` é erro).

**RegularExpressionLiteral (12.9.5)** [LIDO em estrutura]
- Forma: `/` RegularExpressionBody `/` RegularExpressionFlags.
- Body: primeiro caractere não pode ser `*`, `\` nem `/` (que começaria comentário `//` ou `/*`).
  Caracteres seguintes: RegularExpressionChar, escapes com `\` (RegularExpressionBackslashSequence),
  e classes `[...]` (dentro das quais `/` é permitido).
- Flags: sequência de IdentifierPartChar. A validade (d g i m s u v y) e a gramática do padrão
  ficam na seção 22.2.1 (fora do escopo deste analisador léxico).
- Anexo B, padrões legados: ver seção 7.

**Template (12.9.6)** [LIDO]
- NoSubstitutionTemplate: `` ` `` ... `` ` ``.
- TemplateHead: `` `...${ ``. TemplateMiddle: `` }...${ ``. TemplateTail: `` }...` ``.
- TemplateCharacter: qualquer caractere exceto `` ` ``, `\`, `$` seguido de `{`, e LineTerminator é
  normalizado (CR LF e CR viram LF no TV e TRV).
- TemplateEscapeSequence: mesmos escapes de string.
- NotEscapeSequence: escapes inválidos (ex.: `\u` malformado, `\01`) são permitidos só em template
  marcado (tagged). Nesse caso o TV vale `undefined` e o TRV guarda o texto cru.
- TV (valor cozido) e TRV (valor cru) são calculados separadamente.

### 1.11 Contexto de `/` e `}` [LIDO]
- Depois de um token que termina expressão (identificador, literal, `)`, `]`, `}` de expressão, `++`, `--`),
  `/` é divisão.
- Caso contrário, `/` começa RegularExpressionLiteral.
- `}` pode fechar bloco ou fechar template com substituição: o analisador precisa de contexto
  (RightBraceTemplate). Conferir 12.9.6 e 12.8 para o caso.
- Atenção (prática, não é regra da spec): palavras como `return`, `typeof`, `yield`, `await` seguidas
  de `/` abrem regex, não divisão. A spec resolve isso pela gramática (InputElementRegExp no ponto
  de uso), não por lista de palavras. O analisador deve decidir pelo estado da gramática, não por
  heurística.

### 1.12 Inserção automática de ponto e vírgula (12.10) [LIDO parcialmente]
Regras básicas (12.10.1):
1. Se o próximo token não é aceito pela gramática e há quebra de linha antes dele (ou ele é `}`),
   insere `;`. Também vale para `do ... while (...)` no fim da linha.
2. Se o fim da entrada não é aceito, insere `;` no fim.
3. Se uma produção tem "[no LineTerminator here]" e há quebra de linha no ponto marcado, não há
   inserção ali; a produção é interrompida e o `;` é inserido no ponto anterior.
- Nunca insere `;` se ele fizer virar instrução vazia ou se for um dos `;` de cabeçalho de `for`.

Produções com "[no LineTerminator here]" (lista de 12.10.3.2.1 e do texto de 12.10.1) [LIDO]:
- Pós-fixo `++` e `--` (UpdateExpression: `a` `++` só se não há quebra antes do `++`).
- `continue` e `break` com label (sem quebra antes do label).
- `return` (sem quebra antes da expressão).
- `throw` (sem quebra antes da expressão; é erro sintático se houver).
- `yield` e `yield*` (sem quebra antes da expressão).
- Seta `=>` (sem quebra antes de `=>`, na arrow function).
- `async` em `async [no LineTerminator here] function` e `async [no LineTerminator here] ArrowParameters =>`.
- `using` e `await using` em declarações (conferir 14.3.1, marcado [LIDO] pelos erros).

Início de linha que exige `;` antes (12.10.3.1) [LIDO]: `(`, `[`, template, `+`/`-` unários e literal de regex
não são "continuação" natural da linha anterior; a spec não insere `;` ali. O analisador só insere `;`
quando a gramática não aceita o token.

---

## 2. Expressões (seção 13)

Ordem de precedência (do mais fraco para o mais forte). Associatividade e "não pode misturar":

| Nível | Operadores / produção | Associatividade | Observação |
|---|---|---|---|
| 1 (mais fraco) | `,` (13.16) | esquerda | Expression |
| 2 | atribuição `=`, `op=`, `&&=`, `\|\|=`, `??=`, arrow, yield (13.15, 15.3, 15.9, 14) | direita | arrow e yield entram aqui |
| 3 | condicional `a ? b : c` (13.14) | direita | |
| 4 | `??` (13.13 CoalesceExpression) | esquerda | operandos são BitwiseOR. Misturar com `\|\|` ou `&&` sem parênteses é erro |
| 5 | `\|\|` (13.13) | esquerda | |
| 6 | `&&` (13.13) | esquerda | |
| 7 | `\|` (13.12) | esquerda | |
| 8 | `^` (13.12) | esquerda | |
| 9 | `&` (13.12) | esquerda | |
| 10 | `==` `!=` `===` `!==` (13.11) | esquerda | |
| 11 | `<` `>` `<=` `>=` `instanceof` `in` `#x in obj` (13.10) | esquerda | `in` some em `for` (ver 14.7) |
| 12 | `<<` `>>` `>>>` (13.9) | esquerda | |
| 13 | `+` `-` binário (13.8) | esquerda | |
| 14 | `*` `/` `%` (13.7) | esquerda | |
| 15 | `**` (13.6) | direita | `-a ** b` é erro sem parênteses |
| 16 | unários prefixo `delete void typeof + - ~ ! await` (13.5) e `++x` `--x` (13.4) | (prefixo) | |
| 17 | `x++` `x--` pós-fixo (13.4) | — | sem quebra de linha antes |
| 18 | LHS: `new`, chamada, membro, `?.`, tagged template, `super`, `import()`, `import.meta` (13.3) | esquerda | |
| 19 (mais forte) | primário (13.2) | — | |

Observações fixas:
- Operadores binários de mesmo nível associam pela coluna acima.
- `**` tem o lado esquerdo restrito: UpdateExpression ou parêntese, nunca UnaryExpression solto.
- `??` com `||` ou `&&` sem parênteses é SyntaxError (13.13 Early Errors).
- Precedência de `in` depende do contexto de `for` (14.7.4, parâmetro `[~In]`).

### 2.1 Identificadores (13.1) [LIDO]
- IdentifierReference, BindingIdentifier, LabelIdentifier: Identifier | yield (sem [Yield]) |
  await (sem [Await]).
- Identifier: IdentifierName que não é ReservedWord.
- Early Errors: `arguments` e `eval` como BindingIdentifier em strict; `yield` como nome em strict
  ou em gerador; `await` como nome em módulo ou em função async; nome reservado escrito com escape
  é erro.

### 2.2 Primário (13.2) [LIDO]
- `this` (13.2.1).
- IdentifierReference (13.2.2).
- Literal (13.2.3): null, booleano, numérico, string.
- ArrayLiteral (13.2.4): `[ ]`, `[ , ]` (elisões), `[ a, , b, ...c ]`. Spread `...AssignmentExpression`.
- ObjectLiteral (13.2.5) [LIDO]: `{ }`, `{ chave: v }`, `{ x }` (shorthand), `{ ...obj }` (spread),
  `{ m() {} }`, `{ get x() {} }`, `{ set x(v) {} }`, `{ async m() {} }`, `{ *g() {} }`,
  `{ async *g() {} }`, `{ [expr]: v }` (chave computada), nomes numéricos e string,
  `{ __proto__: v }` (B.3.1: define protótipo; duplicado é erro).
  CoverInitializedName `{ x = 1 }` só é válido se a expressão depois virar padrão de atribuição (13.15.5).
- FunctionExpression, ClassExpression, GeneratorExpression, AsyncFunctionExpression,
  AsyncGeneratorExpression (13.2.6, 15.x) [CONFERIR as formas exatas].
- RegularExpressionLiteral (13.2.7) e TemplateLiteral (13.2.8) [LIDO].
  Early Error de template: limite de elementos (2^32) [LIDO].
- Grouping `( Expression )` (13.2.9) [LIDO]. Não pode ser vazio: `()` só é válido como
  cabeçalho de arrow.
- CoverParenthesizedExpressionAndArrowParameterList (13.2) [LIDO]: `()`, `(a, b,)`,
  `(...x)`, `(...[a])`, `(a, ...b)`. O analisador usa isso para decidir se o parêntese é arrow.

### 2.3 Expressões de membro, chamada e metapropriedades (13.3) [LIDO parcial, resto CONFERIR]
- Membro: `a.b`, `a[b]`, `a.#x`, `new M(args)`, `new M` (sem args), `new.target`.
- `super.x` e `super[x]` (SuperProperty).
- `super(args)` (SuperCall, só em construtor de classe derivada).
- Tagged template: `f\`x\`` (13.3.11), e `a.b\`x\`` (MemberExpression tagged).
- Chamada: `f(args)`, args com spread `f(...xs)`.
- `import(expr)` e `import(expr, options)` (13.3.10, ImportCall; o segundo argumento é import attributes) [CONFERIR a versão atual].
- `import.meta` (13.3.12, só em Module) e `new.target` (13.3.12, só dentro de função).
- Cadeia opcional (13.3.9): `a?.b`, `a?.[x]`, `a?.(args)`, `a?.#x`. Não pode ter tagged template em cadeia.
  `new a?.b()` é erro.
- Early Errors principais [LIDO em parte]:
  - `super.x` fora de método: erro. `super()` fora de construtor de classe derivada: erro.
  - `new.target` fora de função: erro (arrow no topo também).
  - `import.meta` fora de módulo: erro.
  - Cadeia opcional com template: erro.

### 2.4 Update (13.4) [CONFERIR]
- Prefixo `++x`, `--x`; pós-fixo `x++`, `x--` (sem quebra antes do operador).
- Early Errors: alvo tem que ser alvo simples (não literal, não chamada); em strict, não pode ser
  `eval` ou `arguments`.

### 2.5 Unários (13.5) [CONFERIR]
- `delete x`, `void x`, `typeof x`, `+x`, `-x`, `~x`, `!x`, `await x`.
- Early Errors: `delete identificador` em strict; `delete obj.#x` (campo privado) é erro.

### 2.6 Exponenciação (13.6) [CONFERIR]
- `x ** y`, associativo à direita. Lado esquerdo não pode ser UnaryExpression sem parênteses.

### 2.7 Multiplicativo, aditivo, deslocamento (13.7, 13.8, 13.9) [CONFERIR]
- `* / %`, `+ -` (binário), `<< >> >>>`.

### 2.8 Relacional (13.10) [CONFERIR]
- `< > <= >= instanceof in`.
- `#x in obj` (ergonomic brand check): PrivateIdentifier antes de `in`, válido só dentro de classe
  que declara `#x`.

### 2.9 Igualdade (13.11), bit a bit (13.12), lógicos (13.13) [CONFERIR]
- Igualdade: `== != === !==`.
- Bit a bit binário: `& ^ |`.
- Lógicos: `&&`, `||`, `??` (regra de mistura da tabela 2).

### 2.10 Condicional (13.14) [CONFERIR]
- `a ? b : c`, com `b` como AssignmentExpression[+In] e `c` como AssignmentExpression.

### 2.11 Atribuição (13.15) [LIDO parcial]
- `LHS = AssignmentExpression`.
- `LHS op= AssignmentExpression`, com op em `+ - * / % ** << >> >>> & | ^`.
- Atribuição lógica: `LHS &&= v`, `LHS ||= v`, `LHS ??= v`. LHS tem que ser alvo simples (não padrão).
- Atribuição com padrão: `[a, b] = arr`, `({a, b} = obj)` (13.15.5, a conferir).
- Early Errors [LIDO]: em `=`, se o LHS é literal de objeto/array, deve cobrir AssignmentPattern;
  nos compostos, alvo tem que ser válido; nas lógicas, alvo tem que ser simples.

### 2.12 Vírgula (13.16) [CONFERIR]
- `Expression : Expression , AssignmentExpression` (sequência).

### 2.13 yield (14.4 e 15.5, ver 15) [CONFERIR]
- `yield`, `yield AssignmentExpression`, `yield* AssignmentExpression`. Só dentro de gerador
  (ou async gerador). `yield` sem valor não aceita quebra de linha antes do valor (ASI, seção 1.12).
- Early Errors: `yield` como nome em gerador ou em strict é erro; `yield` em parâmetro de gerador é erro.

### 2.14 await (13.5 e 15.8, 15.9) [CONFERIR]
- `await UnaryExpression`. Só dentro de função async, ou no topo de módulo (top-level await).
- Early Errors: `await` como nome em módulo ou função async é erro.

### 2.15 Arrow (15.3) e async arrow (15.9) [CONFERIR]
- `x => corpo`, `(a, b) => corpo`, `() => corpo`, `(a = 1, ...r) => corpo`, `({a}) => corpo`.
- Corpo: AssignmentExpression ou `{ ... }` (bloco).
- `async x => corpo`, `async (a) => corpo` (async arrow).
- Sem quebra de linha antes de `=>` (ASI, seção 1.12).
- Early Errors: parâmetros duplicados são erro; `yield`/`await` em parâmetros de arrow é erro conforme contexto.

---

## 3. Comandos e declarações (seção 14)

### 3.1 Estrutura geral
- StatementList e StatementListItem: Statement | Declaration. Declaração (14.1) = HoistableDeclaration |
  ClassDeclaration | LexicalDeclaration. [LIDO na lista de produções de Statement/Declaration]

### 3.2 Bloco (14.2) [LIDO]
- `{ StatementList? }`.
- Early Errors (14.2.1): nomes lexicais duplicados no mesmo bloco são erro (exceção: declarações de
  função em bloco no modo legado do navegador, B.3.3); conflito entre nome lexical e nome `var`
  do mesmo escopo é erro.

### 3.3 Declarações de variável (14.3) [LIDO]
- `var` (14.3.2 VariableStatement): `var a, b = 1, [c] = arr, {d} = obj;`.
- `let` e `const` (14.3.1 LexicalDeclaration): `let a;`, `let a = 1, b;`, `const a = 1;`.
- `using` e `await using` (declaração de recurso; presente no texto de 14.3.1.1) [LIDO nos erros].
- Early Errors (14.3.1.1) [LIDO]: `let` não pode ser nome ligado; nomes duplicados em `let/const/using`
  são erro; `const` exige inicializador; `using` exige inicializador.
- VariableDeclaration (14.3.2): Identifier [Initializer] | BindingPattern Initializer.

### 3.4 Instrução vazia e expressão (14.4, 14.5) [LIDO]
- `;` (EmptyStatement).
- ExpressionStatement: Expression `;`, com lookahead proibido: não pode começar com `{`, `function`,
  `async function`, `class`, `let [`, nem `@` (decorador, se aplicável). [CONFERIR a lista exata.]

### 3.5 if (14.6) [LIDO em estrutura]
- `if (E) S`, `if (E) S else S`.
- Early Errors: função rotulada como cláusula é erro, salvo B.3.4 (função em if no modo legado, não strict).

### 3.6 Laços (14.7) [LIDO]
- do-while (14.7.2): `do S while (E);` (com ASI especial no fim, seção 1.12).
- while (14.7.3): `while (E) S`.
- for (14.7.4) [LIDO]: três formas de inicialização:
  - `for (Expression?; Expression?; Expression?) S`
  - `for (var DeclarationList; ...) S`
  - `for (LexicalDeclaration ...) S` (`let` ou `const`).
  Parâmetro `[~In]` na inicialização (o `in` não é operador ali).
- for-in e for-of e for-await (14.7.5) [LIDO]:
  - `for (LHS in E) S`, `for (var ForBinding in E) S`, `for (ForDeclaration in E) S`
  - mesmas três formas com `of`.
  - `for await (LHS of E) S` (só em async ou top-level de módulo).
  - Early Errors: LHS literal de objeto/array deve cobrir padrão; ForDeclaration com `let` como nome
    é erro; nomes duplicados e conflito com var são erro; função rotulada como corpo é erro.

### 3.7 Controle de fluxo [CONFERIR]
- `continue;`, `continue label;` (14.8). Early Errors: continue fora de laço é erro; label
  que não rotula laço é erro.
- `break;`, `break label;` (14.9). Early Errors: break fora de laço/switch sem label é erro; label
  inexistente é erro.
- `return;`, `return expr;` (14.10). Early Error: return fora de função é erro (em Script).
- `with (E) S` (14.11). Early Error: proibido em strict.
- `switch (E) { case E: ...; default: ... }` (14.12). Early Errors: mais de um `default` é erro;
  declarações lexicais no bloco do switch seguem regra de bloco.
- Rótulo: `label: Statement` (14.13). Early Errors: label duplicado aninhado é erro; `continue label`
  só vale para label de laço; função rotulada é erro em strict (e em geral, salvo B.3.2).
- `throw expr;` (14.14). Early Errors: nenhuma especial além de quebra de linha (ASI, seção 1.12: `throw` sem valor é erro).
- try (14.15): `try B catch (P) B`, `try B finally B`, `try B catch (P) B finally B`,
  e `catch { }` sem binding (ES2019). Early Errors: nomes do parâmetro conflitam com lexicais do bloco.
  B.3.5 permite `var` no catch com mesmo nome (legado).
- `debugger;` (14.16). Sem early error específica.

---

## 4. Funções e classes (seção 15)

Conteúdo da seção 15 não foi lido no texto da spec. Esta seção é **[CONFERIR]** em bloco. Lista de
produções a conferir (números são os da spec):

- 15.1 Parameter Lists: FormalParameters, FormalParameter, parâmetro com valor padrão, rest `...x`
  (só último, sem padrão), desestruturação em parâmetro. Early Errors: duplicados proibidos em
  strict, arrow e parâmetros não simples; `yield`/`await` em parâmetros conforme contexto.
- 15.2 Function Definitions: `function name(params) { body }`, `function (params) { body }` (expressão
  e declaração). Corpo é FunctionBody (com directive prologue, "use strict").
- 15.3 Arrow Function Definitions: seção 2.15 deste documento.
- 15.4 Method Definitions: métodos de objeto e de classe, get, set, async, generator, computados,
  nomes privados `#m()` em classe.
- 15.5 Generator Function Definitions: `function* g() {}`, `yield`.
- 15.6 Async Generator Function Definitions: `async function* g() {}`, `for await` e `yield` juntos.
- 15.7 Class Definitions: `class A extends B { ... }`, membros:
  - método, `static` método, getter/setter, `async`, `*` gerador, `async *`.
  - campos públicos `x = 1;`, `static x = 1;`, campos privados `#x = 1;`, `static #x`.
  - métodos privados `#m() {}`, getters/setters privados.
  - blocos estáticos `static { ... }`.
  - ponto e vírgula vazio `;` dentro da classe.
  - `accessor` (auto-accessor): [CONFERIR se está na edição atual ou ainda é proposta de decorators].
  - Early Errors típicos: construtor duplicado; construtor como getter/setter/gerador/async/campo é erro;
    `#x` não declarado é erro; `#x` declarado duas vezes é erro (exceto par get/set); `prototype`
    como membro estático é erro; `super()` só em construtor de derivada; corpo de classe é sempre strict.
- 15.8 Async Function Definitions: `async function f() {}`, `await`.
- 15.9 Async Arrow Function Definitions: `async (x) => ...`, `async x => ...`.

### 4.1 Modo estrito (11.2.2, diretiva "use strict") [CONFERIR]
- Diretiva "use strict" no início do script ou de função (directive prologue) ativa strict.
- Corpo de classe é sempre strict.
- O que vira erro de sintaxe em strict (principais): `with`; octal legado (`017`, `"\7"`, `"\8"`);
  `delete identificador`; nomes `eval` e `arguments` como alvo de atribuição ou binding; nomes
  `implements interface let package private protected public static yield` como identificador;
  parâmetros duplicados; função rotulada; declaração de função fora de bloco permitida no legado
  (B.3.3, não vale em strict).
- Parte do que é "em strict" é semântica (tempo de execução), não sintaxe: o analisador marca só
  os erros de sintaxe.

---

## 5. Scripts e módulos (seção 16)

Conteúdo da seção 16 não foi lido no texto da spec. Lista a conferir:

### 5.1 Script (16.1) [CONFERIR]
- Goal symbol Script: sem `import`/`export` de módulo, sem `await` de topo, sem `import.meta`.
- `import(expr)` (dinâmico) é permitido em script.

### 5.2 Module (16.2) [CONFERIR]
- ModuleItem: ImportDeclaration | ExportDeclaration | StatementListItem.
- Top-level `await` permitido (13.5 e 16.2).
- Modo estrito sempre (módulo é strict).

### 5.3 ImportDeclaration [CONFERIR as formas]
- `import "mod";` (side-effect)
- `import x from "mod";` (default)
- `import * as ns from "mod";` (namespace)
- `import { a, b as c } from "mod";` (named)
- `import x, { a } from "mod";`, `import x, * as ns from "mod";`
- `import { "string name" as x } from "mod";` (ModuleExportName com string, ES2022)
- Atributos: `import x from "mod" with { type: "json" };` (WithClause, `with` é a forma atual; `assert` foi
  retirado). Chaves do atributo: IdentifierName ou StringLiteral; valores: StringLiteral; chaves duplicadas
  são erro. [CONFERIR a versão atual]
- Early Errors: nomes ligados duplicados no mesmo módulo; `import eval/arguments` em strict.

### 5.4 ExportDeclaration [CONFERIR as formas]
- `export * from "mod";`
- `export * as ns from "mod";` (ES2020)
- `export { a, b as c };`, `export { a } from "mod";`, `export { "string" as x } from "mod";`
- `export var/let/const/function/class/async function ...;` (declaração exportada)
- `export default expr;` (AssignmentExpression)
- `export default function f() {}`, `export default function () {}`
- `export default class C {}`, `export default class {}`
- `export default async function f() {}`
- Early Errors: exporta nome não declarado; exporta duas vezes o mesmo nome; mais de um `export default`;
  `export` fora de módulo é erro.

### 5.5 Nomes de módulo (ModuleExportName) [CONFERIR]
- IdentifierName | StringLiteral (string precisa ser Unicode bem-formada, sem surrogate solto).

---

## 6. Desestruturação (13.15.5 para atribuição; 8.x/14.3 para ligação) [CONFERIR]

### 6.1 Padrão de ligação (BindingPattern)
- ObjectBindingPattern: `{ a, b: c, d = 1, ...rest }`, `{ [expr]: x }`, `{ "s": x }`.
- ArrayBindingPattern: `[a, , b = 1, ...rest]`.
- Pode aparecer em: declaração `var/let/const`, parâmetro de função, catch (`catch ({ message })`), for-in/of.
- Rest em padrão de objeto: só como último; em array: só como último.

### 6.2 Padrão de atribuição (AssignmentPattern)
- ObjectAssignmentPattern: `{ a, b: c.d, ...rest }`, alvos podem ser membros.
- ArrayAssignmentPattern: `[a.b, , c]`.
- Aparece em `x = y` e em `for (pattern of ...)`.
- Early Errors: alvo tem que ser válido (identificador ou membro, não literal); `eval`/`arguments` em strict.

---

## 7. Anexo B (recursos adicionais para navegadores) [CONFERIR todo o anexo]

Não foi lido o texto do Anexo B. Lista dos itens a conferir:
- B.1.1 Comentários HTML-like (`<!--` e `-->`, seção 1.4 deste documento).
- B.1.2 Literais numéricos e strings legados: octal `017`, `08`, `\1` a `\377`, `\8` e `\9`.
- B.1.4 Padrões de regex legados (padrões de Regex que a gramática normal recusa, ex.: `\c` isolado,
  quantificador `{` sem fechamento, classes com escapes diferentes, `]` solto em alguns contextos).
  Em modo `u`/`v` estes padrões não valem.
- B.3.1 `__proto__` em literal de objeto: `{ __proto__: x }` define protótipo; duplicado é erro.
- B.3.2 Função rotulada: `label: function f() {}` (não strict).
- B.3.3 Função em bloco no modo não estrito (web legacy): declaração de função dentro de bloco é
  visível fora dele (regra de compatibilidade). Não vale em strict.
- B.3.4 Função como cláusula de `if` (não strict).
- B.3.5 `var` com mesmo nome do parâmetro de `catch` (não strict).
- B.3.6 Inicializador em `for (var x = 1 in obj)` (não strict).

---

## 8. Erros de sintaxe mais frequentes (lista para testes de regressão do analisador)

1. `?? ` misturado com `||` ou `&&` sem parênteses (13.13).
2. `-a ** b` sem parênteses (13.6).
3. `new a?.b()` (13.3.9).
4. Template em cadeia opcional `a?.b\`x\`` (13.3.9).
5. `super` fora de contexto permitido (13.3, 15.4, 15.7).
6. `new.target` fora de função (13.3.12).
7. `import.meta` em script (13.3.12, 16.1).
8. `await` como nome em módulo ou async (13.1).
9. `yield` como nome em gerador ou strict (13.1).
10. `let` como nome em declaração lexical (14.3.1.1).
11. Declaração lexical duplicada no mesmo bloco (14.2.1).
12. `const` sem inicializador (14.3.1.1).
13. `continue` fora de laço, `break` fora de laço ou switch (14.8, 14.9).
14. `return` fora de função (14.10).
15. `with` em strict (14.11).
16. Mais de um `default` no switch (14.12).
17. Label inexistente ou duplicado (14.13).
18. Catch: parâmetro conflitando com lexical do bloco (14.15).
19. Parâmetros duplicados em strict, arrow ou não simples (15.1).
20. Construtor duplicado ou não permitido em classe (15.7).
21. `#x` não declarado ou declarado duas vezes (15.7, 13.10).
22. Export duplicado, export de nome não declarado, dois `export default` (16.2).
23. Quebra de linha entre `=>` e os parâmetros (ASI, seção 1.12).
24. Numéricos inválidos: `3in`, `1__0`, `0_1`, `1_`, `0x_1` (12.9.3).
25. String com LF cru (12.9.4).
26. Template com escape inválido fora de tagged (12.9.6).
27. Regex com flag inválida, com `/` não escapado fora de classe, ou body vazio `//` (12.9.5, 22.2.1).

---

## 9. Lista sugerida de tipos de nó da árvore (sugestão, em português)

Convenção sugerida: um tipo por produção relevante. Nomes com `Expressao::`, `Comando::`,
`Declaracao::`, `Padrao::`, `Funcao::`, `Classe::`, `Modulo::`. Só nomes, sem código Rust.

Léxico (não viram nó; o analisador produz tokens):
- Token::Identificador, Token::Palavra, Token::Pontuador, Token::Numero, Token::BigInt,
  Token::Texto, Token::Template, Token::Regex, Token::PrivadoId, Token::Fim

Expressões (13):
- Expressao::Identificador (13.1, 13.2.2)
- Expressao::Este (13.2.1)
- Expressao::Literal (13.2.3): Nulo, Booleano, Numero, BigInt, Texto
- Expressao::Regex (13.2.7)
- Expressao::Template (13.2.8) e Expressao::TemplateMarcado (13.3.11)
- Expressao::Lista (13.2.4) e Expressao::Vetor com elisao e spread
- Expressao::Objeto (13.2.5) com Propriedade::Valor, Propriedade::Atalho, Propriedade::Metodo,
  Propriedade::Getter, Propriedade::Setter, Propriedade::Espalhar, Propriedade::Computada,
  Propriedade::ProtoDireto (B.3.1)
- Expressao::Agrupada (13.2.9)
- Expressao::Funcao, Expressao::Classe, Expressao::Gerador, Expressao::FuncaoAssincrona (13.2.6)
- Expressao::Membro (13.3, com computado e privado), Expressao::MembroPrivado
- Expressao::Chamada (13.3, com argumentos e espalhamento)
- Expressao::Novo (13.3)
- Expressao::SuperMembro, Expressao::SuperChamada (13.3.7)
- Expressao::NovoAlvo (13.3.12), Expressao::MetaImportar (13.3.12)
- Expressao::ImportarDinamico (13.3.10)
- Expressao::CadeiaOpcional (13.3.9), com Acesso/Chamada opcionais
- Expressao::Atualizacao (13.4) com prefixo e pós-fixo
- Expressao::Unaria (13.5): Deletar, Vazio, Tipo, Mais, Menos, Negar, Inverter, Nao, Aguardar
- Expressao::Exponencial (13.6)
- Expressao::Multiplicativa (13.7), Expressao::Aditiva (13.8), Expressao::Deslocamento (13.9)
- Expressao::Relacional (13.10), Expressao::EmBrancoPrivado (`#x in`)
- Expressao::Igualdade (13.11)
- Expressao::BitwiseBinaria (13.12)
- Expressao::Logica (13.13): E, Ou, Coalescencia
- Expressao::Condicional (13.14)
- Expressao::Atribuicao (13.15) com Alvo simples ou Padrao; Expressao::AtribuicaoLogica
- Expressao::Sequencia (13.16)
- Expressao::Seta (15.3) e Expressao::SetaAssincrona (15.9)
- Expressao::Rendimento (15.5) e Expressao::RendimentoDelegado (`yield*`)
- Expressao::Aguardar (13.5, 15.8)

Comandos (14):
- Comando::Bloco (14.2)
- Comando::Vazio (14.4)
- Comando::Expressao (14.5)
- Comando::Se (14.6)
- Comando::Enquanto (14.7.3), Comando::FacaEnquanto (14.7.2)
- Comando::Para (14.7.4), Comando::ParaEm (14.7.5), Comando::ParaDe (14.7.5), Comando::ParaAguardar (14.7.5)
- Comando::Continuar (14.8), Comando::Interromper (14.9), Comando::Retornar (14.10)
- Comando::Com (14.11)
- Comando::Escolher (14.12), Comando::Caso (14.12)
- Comando::Rotulado (14.13)
- Comando::Lancar (14.14)
- Comando::Tentar (14.15), com Comando::Capturar e Comando::Finalmente
- Comando::Depurar (14.16)

Declarações (14.3):
- Declaracao::Variavel (var) (14.3.2) com Declarador
- Declaracao::Lexica (let, const, using) (14.3.1)
- Declaracao::Funcao (15.2, 15.5, 15.6, 15.8)
- Declaracao::Classe (15.7)

Padrões (13.15.5, 14.3):
- Padrao::Objeto, Padrao::Vetor, Padrao::Ligacao (identificador), Padrao::Alvo (membro, em atribuição),
  Padrao::Espalhar (rest), Padrao::Padrao (valor padrão `= x`)

Funções e classes (15):
- Funcao::Parametros (15.1), Funcao::Corpo (15.2), Funcao::Metodo (15.4),
  Funcao::Gerador (15.5), Funcao::Assincrona (15.8), Funcao::AssincronaGerador (15.6)
- Classe::Corpo (15.7), Classe::Membro, Classe::Metodo, Classe::Campo (público e privado, com `static`),
  Classe::BlocoEstatico (15.7), Classe::Construtor, Classe::Acessor (se existir na versão atual)

Módulos (16):
- Modulo::Importar (16.2, ImportDeclaration): Lado padrão, Namespace, Nomeado, Efeito, Atributos
- Modulo::Exportar (16.2, ExportDeclaration): Nomeado, Todos, TodosComo, Padrao, Declaracao, De
- Modulo::NomeExportado (ModuleExportName: identificador ou texto)
- Modulo::Atributos (WithClause)

Programa:
- Programa::Script (16.1)
- Programa::Modulo (16.2)

Observação para o analisador: a árvore deve guardar a posição (início e fim, e linha/coluna) em cada
nó, porque os erros de early error precisam apontar o trecho. Isto é sugestão de projeto, não da spec.

---

## 10. O que ficou de fora ou precisa conferir antes de codificar

- Seções 12.1, 12.8 (lista completa de OtherPunctuator), 12.9.3 (regras exatas do `_`), 12.9.4 (lista
  exata de escapes e LineContinuation), 12.9.5 (gramática de classes de caracteres na regex) e 12.10.2
  (exemplos de ASI) precisam de leitura direta da spec.
- Seções 13.4 a 13.16 (operadores), exceto a precedência da tabela da seção 2, precisam de leitura direta.
- Seção 15 inteira e seção 16 inteira precisam de leitura direta. Esta é a maior lacuna: são as
  partes com mais formas de sintaxe (classes, campos, módulos, atributos de import).
- Anexo B inteiro precisa de leitura direta.
- Recursos de versão recente (import attributes, `using`, `accessor`, modificadores de regex, nomes
  duplicados de grupo em regex) precisam ser confirmados na edição atual; aqui aparecem como
  [CONFERIR] ou como [LIDO] apenas quando o texto trazido os confirmou.
- Regex: a gramática interna do padrão (22.2.1) fica fora do analisador léxico. Planejar separadamente.
