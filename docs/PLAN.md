# jlock — plano completo

Serviço de travas por chave (arquivo, pasta ou nome) para a máquina inteira, como um CLI em Rust, sem servidor.
Um dono por chave; fila em ordem de chegada; dois pedidos simultâneos decididos por uma operação atômica do
sistema de arquivos; travas de processo morto recuperadas por heartbeat; deadlock detectado; lista de tudo que está
travado e por quem; `unlock --force`; zero arquivos sobrando quando nada está travado.

Primeiro usuário: o goal-tasks (plano em `D:\tools\goal-tasks\plano.html`), que coordena agentes editando o mesmo
checkout em paralelo. Mas o jlock não sabe o que é tarefa nem execução: serve para scripts, builds, deploys e para
qualquer agente (Claude Code, OpenCode, Codex, pi) por shell.

Data do plano: 23/10/2026. Revisão 3 (Rust, identidade genérica, zero sobras, nota multiplataforma).

---

## 1. Por que existe

- As ferramentas de edição dos agentes abrem, gravam e fecham um arquivo em milissegundos. A trava que o Windows
  dá ao abrir um arquivo (modo de compartilhamento) vale só enquanto o handle está aberto, é obrigatória para todos
  (travaria o editor do usuário), morre com o processo e não tem fila nem identidade. A posse de que o goal-tasks
  precisa dura a tarefa inteira (minutos), atravessa dezenas de aberturas e processos que nascem e morrem.
- O mutex nomeado do Windows precisa de um processo vivo segurando o handle, não tem fila com ordem garantida e
  não lista quem tem o quê.
- Em Linux, `flock(1)` + `lslocks` cobrem parte (segurar durante um comando, liberação pelo kernel, lista por PID),
  mas sem ordem de chegada garantida, sem identidade de tarefa, sem deadlock, sem segurar entre invocações sem
  processo residente. Lá o jlock é conveniência; no Windows é necessidade. O mesmo binário serve nos três sistemas.
- Worktrees com copy-on-write (Btrfs, XFS, ZFS; não ext4; ReFS só no Windows 11) barateiam o disco, mas não
  resolvem o merge nem as órfãs. A escolha por posse de arquivos não depende do sistema operacional.

### O que existe e não serve inteiro

| Opção | Dá | Falta |
|---|---|---|
| `LockFileEx` (Windows) | trava exclusiva, bloqueante; o SO libera na morte | sem ordem; sem lista; precisa de FFI em linguagens de script |
| `flock(1)` + `lslocks` (Linux) | segurar durante um comando; liberação pelo kernel; lista por PID | não existe no Windows; sem ordem garantida; sem identidade; sem deadlock |
| `proper-lockfile` (npm) | `mkdir` atômico, heartbeat por mtime, trava morta, "trava comprometida" | sem fila (tentativas ao acaso); sem CLI; sem lista global; 2020 |
| GAIA PR #5469 (AMD) | desenho de fila justa entre processos (uma senha por esperador) | não é ferramenta; sem deadlock |

Referências: https://github.com/moxystudio/node-proper-lockfile, https://github.com/amd/gaia/pull/5469,
https://devblogs.microsoft.com/oldnewthing/20260430-00/?p=112288%2F (fairness em travas no Windows).

---

## 2. Primitivas verificadas (23/10/2026, NTFS, Windows 10)

Testadas em Bun 1.3.14 por conveniência (scripts em `docs/evidence/`); são chamadas do sistema
(`CreateFile` com `CREATE_NEW`, `MoveFileEx`, `DeleteFile`) e valem igual para Rust, que as chama direto.
Os testes serão refeitos em `cargo test`.

| Operação | Resultado | Uso |
|---|---|---|
| criar arquivo exclusivamente (`O_EXCL` / `CREATE_NEW`), 12 processos competindo, 30 rodadas | sempre exatamente 1 vencedor; perdedores com `EEXIST`; conteúdo íntegro | decidir quem pega a trava; emitir senhas únicas |
| `mkdir` repetido | `EEXIST` | equivalente; não usado |
| renomear por cima de arquivo existente fechado | funciona | quebrar trava morta; substituir arquivos derivados |
| apagar arquivo que outro processo lê (aberto com compartilhamento de exclusão) | funciona | liberar com leitores concorrentes |
| arquivo preso por programa sem compartilhamento de exclusão (editor, antivírus) | não testado; o Windows recusa com `EBUSY`/`EPERM` | tentativas com espera crescente; nunca matar processos |

Em Rust: `OpenOptions::new().write(true).create_new(true)` é a criação exclusiva, atômica em NTFS, ext4, Btrfs e APFS.

---

## 3. Decisões

1. **CLI independente, instalado no PATH, para qualquer uso.** Repositório próprio (`D:\jlock`). O goal-tasks o
   chama como processo, lendo `--json` e os códigos de saída. Não há segunda implementação do protocolo.
2. **Rust, binário estático, sem runtime.** Chamado dezenas de vezes por minuto; um `hold` residente por tarefa.
   Partida em milissegundos, 1–2 MB por processo. Primitivas do SO sem FFI. `cargo install --path .` deixa
   `jlock.exe` em `~\.cargo\bin` (já no PATH). Builds Linux/macOS pelo CI.
3. **Registro em um lugar comum do disco, sem servidor.** `%LOCALAPPDATA%\jlock\` no Windows,
   `$XDG_STATE_HOME/jlock` ou `~/.local/state/jlock` em Linux, `~/Library/Application Support/jlock` em macOS;
   `JLOCK_HOME` sobrepõe. Nenhum processo a manter vivo: se qualquer coisa cair, o estado está no disco.
4. **Um arquivo por trava, não um arquivo único.** "Consultar o arquivo e depois registrar" são dois passos e dois
   pedidos simultâneos leriam "livre" os dois. Criar `holder.json` exclusivamente é consulta e registro numa só
   operação. A lista de tudo que está travado é a leitura da pasta; `index.json` é derivado, nunca a verdade.
5. **Fila estrita por senha.** Quem quer a trava cria uma senha (hora em microssegundos + aleatório). Só o dono
   da menor senha viva tenta pegar; os outros esperam. Ordem de chegada de verdade.
6. **Heartbeat para dono e esperadores.** Renovação do mtime a cada 5 s; sem renovação por 30 s está morto:
   qualquer participante quebra a trava ou descarta a senha, com registro. Dono que descobre a trava quebrada
   recebe `lost` e para.
7. **Deadlock detectado.** O grafo "quem espera por quem" está na pasta. Pedido que fecharia um ciclo é recusado
   com a cadeia; num ciclo formado por pedidos simultâneos, a senha mais nova desiste.
8. **Chaves são caminhos absolutos normalizados ou nomes.** Caminho → resolvido, minúsculas (em sistemas
   insensíveis a caixa), barras `/`; trava em diretório conflita com travas em qualquer caminho dentro dele e
   vice-versa. Nome (`name:porta-3000`) → igualdade exata. Chave especial `*` conflita com tudo.
9. **Identidade genérica: `owner`, `group`, `intent`, `meta`, `ttl`.** O jlock não interpreta nenhum deles.
10. **`unlock --force` com registro de quem forçou.** O dono descobre no próximo heartbeat e para.
11. **Zero sobras.** Todo arquivo tem um dono responsável por apagá-lo e um prazo de expiração. A pasta da chave
    some quando a fila esvazia. Só o log cresce, rotacionado por tamanho.
12. **Trava consultiva.** Só quem usa o jlock a respeita. Para agentes, as guardas do goal-tasks forçam o respeito.
13. **Conjuntos em ordem canônica.** `acquire` com várias chaves pega todas ou nenhuma, em ordem; isso impede
    deadlock entre participantes que só usam posse declarada.

---

## 4. Layout no disco

```
%LOCALAPPDATA%\jlock\
├─ locks\
│  └─ 3f9a…c21\                     ← uma chave: sha1 da chave normalizada (hex)
│     ├─ key                         ← a chave em texto ("d:/temp/spybrowser/src/humanizer/types.ts")
│     ├─ holder.json                 ← existe = ocupada; conteúdo = quem; mtime = heartbeat
│     ├─ queue\
│     │   ├─ 1792794410123456-7c1e.json   ← senha: hora-µs + aleatório; conteúdo = quem; mtime = heartbeat
│     │   └─ 1792794411004211-02bd.json
│     └─ broken\                     ← só alvo de rename atômico; nunca fica arquivo nele
├─ index.json                        ← derivado: {chave → dono, fila}; reescrito por rename; só leitura rápida
├─ last-sweep                        ← carimbo da última varredura global
└─ log.jsonl                         ← acquire/release/expire/force/gc; rotação em 5 MB, mantém log.1.jsonl
```

### Corpo de `holder.json` e das senhas

```json
{ "key": "d:/temp/spybrowser/src/humanizer/types.ts",
  "owner": "claude:a8ec2aca",
  "group": "cursory-261023-k7f2",
  "intent": "T04 Mouse move",
  "meta": { "task": "T04", "tool": "claude" },
  "pid": 171828, "host": "RODRIGO-PC", "cwd": "D:/Temp/spybrowser",
  "since": "2026-10-23T23:06:50.123Z",
  "ttl_secs": 7200 }
```

- `owner` obrigatório (padrão `pid:<pid>`); quem apresenta o mesmo `owner` pode liberar.
- `group`, `intent`, `meta`, `ttl` opcionais. `ttl` é prazo máximo de posse mesmo com heartbeat (processo vivo
  mas descontrolado); expira com registro, como a morte.
- `pid`, `host`, `cwd`, `since` preenchidos pelo jlock. PID morto é uma segunda pista de morte (não substitui o
  heartbeat: PID pode ser reutilizado).

### Linha do `log.jsonl`

```json
{ "at": "2026-10-23T23:06:50.123Z", "event": "acquire|release|queued|expired|forced|deadlock|lost|gc",
  "key": "…", "owner": "…", "group": "…", "by": "…", "reason": "…", "waited_ms": 1234 }
```

---

## 5. Normalização de chaves

- Entrada que parece caminho (contém `/`, `\`, ou começa com letra de unidade, `.`, `~`): resolver para absoluto a
  partir do cwd; `~` → home; separadores → `/`; em Windows e macOS, minúsculas (sistemas insensíveis a caixa); em
  Linux, manter caixa; remover barra final; `\\?\` removido. Não exige que o caminho exista.
- Entrada `name:<texto>`: chave literal, sem normalização além de aparar espaços.
- `*`: chave universal.
- Hash: sha1 da chave normalizada, em hex, nome do diretório. O arquivo `key` guarda o texto.
- Conflito hierárquico (só chaves-caminho): `a` conflita com `b` se `a == b`, `a` é prefixo de diretório de `b`
  (`a + "/"` prefixo de `b`) ou vice-versa. `*` conflita com tudo. `name:` só com igual.

---

## 6. O CLI

| Comando | O que faz |
|---|---|
| `jlock acquire <chave…> [--owner id] [--group g] [--intent "…"] [--meta k=v…] [--ttl 2h] [--wait\|--no-wait] [--timeout 30s] [--json]` | pega todas as chaves (ordem canônica, tudo ou nada). Padrão: espera sem limite. Saída: `granted`; `queued` (dono e posição) com `--no-wait` ou timeout; `deadlock` com a cadeia. |
| `jlock run <chave…> [opções de acquire] -- <comando…>` | como `flock(1)`: pega, mantém heartbeat, roda o comando, libera ao terminar (ou ao morrer: expira). Código de saída do comando é repassado. |
| `jlock hold <chave…> [opções de acquire]` | fica vivo renovando o heartbeat; encerra ao receber a linha `release` na entrada padrão, SIGINT/CTRL+C, ou ao ser encerrado. Imprime `granted` e depois eventos (`lost`). |
| `jlock release [<chave…>] [--owner id] [--group g]` | libera (só o dono). Sem chaves: tudo do dono; com `--group`: tudo do grupo. |
| `jlock heartbeat <chave> --owner id` | renova; `lost` se a trava não é mais sua. (Para quem não usa `hold`.) |
| `jlock status <chave>` | dono e fila da chave, inclusive conflitos por hierarquia (pasta acima ou arquivos dentro). |
| `jlock list [--owner x] [--group g] [--under D:\proj] [--stale] [--json] [--watch]` | tudo que está travado na máquina: chave, dono, grupo, intenção, há quanto tempo, heartbeat, fila. `--watch` redesenha a cada segundo. |
| `jlock graph [--json]` | quem espera por quem; ciclos. |
| `jlock unlock (<chave> \| --owner x \| --group g) --force [--reason "…"]` | quebra travas de outrem, registrando quem forçou; o dono recebe `lost` no próximo heartbeat. |
| `jlock gc [--json]` | varredura global agora: expira travas e senhas mortas, consome `broken\`, remove pastas de chave vazias, rotaciona o log, reescreve o índice; imprime o que removeu. |
| `jlock --version`, `jlock --help` | — |

### Códigos de saída

| Código | Significado |
|---|---|
| 0 | concedido / ok |
| 10 | ocupado (ficou na fila; só com `--no-wait`) |
| 11 | timeout esperando |
| 12 | deadlock |
| 13 | não é o dono (release/heartbeat) |
| 14 | trava perdida (`hold`/`run` terminam com 14 se o dono descobriu `lost`) |
| 2 | uso incorreto (argumentos) |
| 20 | erro de sistema de arquivos (permissão, disco cheio, arquivo preso) |

Em `run`, o código do comando é repassado quando a trava foi concedida; os códigos acima aparecem só quando a
trava não foi obtida ou foi perdida.

### Saída `--json`

Um objeto por comando, com `result` (`granted|queued|timeout|deadlock|released|not_owner|lost|ok|error`) e os
campos pertinentes (`key`, `keys`, `owner`, `holder`, `position`, `chain`, `waited_ms`, `error`). Sem `--json`,
uma linha curta por chave.

### Parâmetros de tempo (configuráveis por variáveis de ambiente, com padrão)

| Variável | Padrão | Uso |
|---|---|---|
| `JLOCK_HOME` | por SO (seção 3.3) | raiz do registro |
| `JLOCK_HEARTBEAT_MS` | 5000 | intervalo de renovação |
| `JLOCK_STALE_MS` | 30000 | sem renovação por isso = morto |
| `JLOCK_POLL_MS` | 250 | intervalo base de sondagem (com jitter de ±40%) |
| `JLOCK_SWEEP_MS` | 60000 | intervalo mínimo entre varreduras globais |
| `JLOCK_LOG_MAX_BYTES` | 5242880 | rotação do log |

Os testes usam valores curtos por essas variáveis.

---

## 7. O protocolo

### acquire(key, owner)

```
(1) hierarquia: coletar chaves-caminho travadas ou com fila que conflitam (ancestrais/descendentes) → a espera
    é na chave do dono conflitante (uma senha em cada chave conflitante, em ordem canônica)
(2) deadlock: grafo = (senhas → donos das chaves) + aresta (owner → dono da chave); ciclo contendo owner → recusar
    { deadlock, chain }
(3) garantir locks/<hash>/ e queue/ ; gravar `key` se não existir
(4) senha ← criar queue/<agora-µs>-<4 hex aleatórios>.json com create_new (conteúdo: corpo da seção 4)
    ENOENT ao criar (a pasta foi apagada por um release neste instante) → voltar a (3)
(5) laço, a cada POLL_MS com jitter:
      renovar mtime da minha senha
      descartar senhas com heartbeat > STALE_MS (renomear para broken/: só um consegue; registrar)
      holder.json com heartbeat > STALE_MS, ou com ttl vencido → renomear para broken/ (só um consegue),
          registrar "expired", apagar o arquivo em broken/
      se minha senha é a menor viva e holder.json não existe:
          criar holder.json com create_new: conseguiu → apagar minha senha → granted
                                             EEXIST → outro passou à frente neste instante; registrar; continuar
      senão → --no-wait, ou timeout vencido → apagar minha senha → queued { holder, position } / timeout
      a cada 8 sondagens: refazer o teste de deadlock; ciclo e minha senha é a mais nova → apagar senha, deadlock
```

Para várias chaves: ordenar as chaves normalizadas; adquirir uma a uma; se alguma falhar (timeout/deadlock),
liberar as já obtidas e devolver qual faltou.

### heartbeat(key, owner)

Ler `holder.json`; `owner` diferente ou arquivo ausente → `lost`; senão renovar mtime (`set_file_times` para agora).

### release(key, owner)

`owner` diferente → `not_owner`; senão apagar `holder.json`; se `queue/` vazia e `broken/` vazia → apagar `key`,
`queue/`, `broken/` e a pasta da chave (`rmdir` falha com `ENOTEMPTY` se alguém acabou de entrar: tudo bem).

### force(key | owner | group)

Renomear `holder.json` para `broken/<ts>-forced.json`, ler, registrar `forced` com `by` e `reason`, apagar.
Senhas não são afetadas; o próximo da fila pega.

### Varredura

- Local, em toda chamada que toca uma chave: expirar senhas e trava mortas, consumir `broken/`, remover a pasta
  se vazia ao sair.
- Global, no máximo uma vez por `SWEEP_MS`: qualquer comando confere `last-sweep`; se venceu, varre `locks/`
  inteira (mesmas ações em cada chave), rotaciona o log, reescreve `index.json`, renova `last-sweep`.
  Concorrência entre varredores é inofensiva: todas as remoções são idempotentes e os renames atômicos.
- `jlock gc`: a global agora, com relatório.

### Escritas

- Criação de trava e senha: `create_new` (atômico).
- Quebra: `rename` (atômico; um só vence).
- `index.json` e rotação do log: escrever `.tmp` e `rename` por cima.
- Tentativas em `EBUSY`/`EPERM`/`EACCES` em operações de remoção/rename: 50, 100, 200, 400, 800, 1600 ms; depois
  erro 20 com a mensagem do SO. Nunca matar processos.

---

## 8. Zero sobras

| Arquivo | Nasce | Morre | Se o responsável morrer | Pior sobra |
|---|---|---|---|---|
| `locks\<hash>\holder.json` | na concessão | `release`, `force`, expiração | expira em STALE_MS; qualquer participante ou `gc` remove | 30 s + até a próxima chamada |
| `queue\<senha>.json` | ao entrar na fila | concessão, timeout, desistência (pelo dono) | expira em STALE_MS; removida por quem passar ou `gc` | idem |
| `broken\…` | só como alvo do rename | milissegundos depois (quem quebrou anota no log e apaga) | o `gc` anota e apaga | até a próxima varredura |
| pasta da chave (`key`) | no primeiro pedido | no `release` que esvazia a fila, ou no `gc` | pasta vazia é removida por qualquer varredura | até a próxima varredura |
| `index.json` | derivado, um só | reescrito por rename | — | um arquivo |
| `log.jsonl` | uma linha por evento | rotação em 5 MB, uma cópia anterior | — | 10 MB |

Em regime, o número de pastas em `locks\` é o número de chaves travadas ou com fila agora; com nada travado,
zero. Invariante testado: 8 processos × 1000 ciclos com mortes provocadas → `locks\` vazia, `broken\`
inexistente, log dentro do limite.

---

## 9. Garantias e limites

| Garantido | Não garantido (e por quê não importa) |
|---|---|
| um dono por chave a qualquer instante | impedir quem não usa o jlock (consultivo; agentes são forçados pelas guardas do goal-tasks) |
| ordem de chegada estrita entre esperadores vivos | entrega instantânea (até POLL_MS) |
| trava de processo morto recuperada em ≤ STALE_MS, com registro | distinguir morto de congelado > STALE_MS (o congelado recebe `lost` e para) |
| deadlock detectado e resolvido deterministicamente | coordenação entre máquinas |
| a lista é sempre a verdade (é a pasta) | — |
| funciona sem processo residente | limpeza sem nenhuma chamada: sobras ficam até a próxima chamada ou `gc` |
| `locks\` vazia quando nada está travado | — |

---

## 10. Falhas e recuperação

| Situação | O que acontece |
|---|---|
| dono morre | heartbeat para; em ≤ STALE_MS qualquer participante ou `gc` quebra a trava com registro; a fila segue |
| esperador morre | senha expira; a fila fecha o buraco |
| dois pedidos no mesmo microssegundo | o aleatório desempata a ordem; `create_new` decide a posse |
| dois participantes quebram a mesma trava morta | rename atômico: um consegue; o outro recebe `ENOENT` e segue |
| dono congelado > STALE_MS mas vivo | trava quebrada; ele vê `lost` e para (código 14) |
| `unlock --force` por engano | `log.jsonl` diz quem, quando, por quê; o dono para em vez de sobrescrever |
| antivírus/editor segurando arquivo do jlock | tentativas com espera crescente; persistiu → código 20, mensagem do SO, sem matar processo |
| relógio salta | pode expirar travas vivas uma vez; donos recebem `lost` e param |
| caminho longo, maiúsculas, barras | chave normalizada; diretório usa o hash |
| disco cheio / sem permissão em `JLOCK_HOME` | erro ≠ `EEXIST` é reportado (20), nunca lido como "ocupado" |
| `index.json` corrompido ou velho | derivado: ignorado e reescrito |
| pasta da chave apagada no instante em que alguém cria a senha | `ENOENT` na senha → recria a pasta e tenta de novo; `rmdir` falha com `ENOTEMPTY` se a senha chegou antes |
| `JLOCK_HOME` apagado com travas ativas | somem para todos; donos recebem `lost` |

---

## 11. Quem usa, e como

| Usuário | Como | Respeito forçado? |
|---|---|---|
| goal-tasks (mod do Claude Code) | `$.process.run(["jlock","acquire",…,"--json"])` em start/claim; `$.process.spawn(["jlock","hold",…])` enquanto a tarefa anda; `release --group` no fim; a guarda em `tool.call` recusa edição fora da posse | sim, pela guarda |
| goal-tasks (plugin do OpenCode) | o mesmo, pelo processo do servidor do plugin; hook de permissão como guarda | sim |
| Codex, pi, outros agentes | skill curta: "antes de editar, `jlock acquire`; ao terminar, `release`"; `jlock run` para comandos | não (depende do modelo) |
| scripts e pessoas | `jlock run D:\proj\dist -- bun run build`; `jlock list --watch` num painel do herdr | — |

---

## 12. Testes (cargo test; processos filhos são do próprio binário de teste, encerrados por PID)

1. Corrida: 12 processos × 30 rodadas pedindo a mesma chave com `--no-wait`; sempre 1 `granted`, os outros
   `queued`/10; conteúdo do `holder.json` íntegro.
2. Ordem de chegada: 8 processos com atrasos conhecidos; concessão na ordem das senhas.
3. Hierarquia: `D:\p\src` travada faz `D:\p\src\a.ts` esperar e vice-versa; `D:\p\docs` não espera.
4. Nomes: `name:x` conflita só com `name:x`; `*` com tudo.
5. Dono morto / esperador morto (filhos nossos encerrados por PID; STALE curto por env): recuperação; `broken\`
   vazio depois; log registra `expired`.
6. Trava perdida e `unlock --force`: `hold` termina com 14; log registra `forced` com `by`/`reason`.
7. Deadlock: A segura x e pede y; B segura y e pede x → 12 com a cadeia; variante simultânea → a senha mais
   nova desiste.
8. Conjuntos: {a,b} e {b,a} nunca travam.
9. `run`: o comando roda com a trava; matar o `run` (filho nosso) → a trava expira.
10. `list`/`index.json`: lista bate com a pasta mesmo com o índice apagado.
11. Zero sobras: 8 processos × 1000 ciclos com mortes provocadas → `locks\` vazia, `broken\` inexistente, log
    dentro do limite.
12. Windows: `release` com `list --watch` lendo ao mesmo tempo; chaves com maiúsculas e barras invertidas
    normalizam igual; caminho relativo e absoluto para o mesmo arquivo dão a mesma chave.
13. Normalização Linux/macOS: testes unitários puros de `normalize_key` por plataforma (sem processos).

---

## 13. Entrega (ordem dos tickets em `.scratch/jlock/issues/`)

1. Esqueleto do crate, `JLOCK_HOME`, normalização de chaves, `status` numa chave livre.
2. `acquire`/`release` sem fila (criação exclusiva), zero sobras da pasta da chave, teste de corrida.
3. Fila por senha: `--wait`/`--no-wait`/`--timeout`, códigos de saída, `--json`.
4. Heartbeat, expiração, `broken\`, `hold`, `run`, `ttl`, `lost`.
5. Hierarquia de caminhos, `name:`, `*`, conjuntos em ordem canônica, `status` com conflitos.
6. Deadlock: grafo, recusa antes da fila, verificação periódica, `graph`.
7. `list` (filtros, `--json`, `--watch`), `unlock --force`, `release --group`, `log.jsonl` com rotação,
   `index.json`.
8. Varredura local e global, `gc`, teste de zero sobras.
9. Trava do SO por baixo de `hold`/`run` (`LockFileEx` / `flock`), liberação imediata pelo kernel.
10. Empacotamento: README de uso, `cargo install --path .`, verificação do binário no PATH, CI com matriz
    Windows/Linux/macOS.

Tamanho estimado: núcleo 500–700 linhas, CLI 300, mais testes.

### Opcional, depois

- Painel: `jlock list --watch` numa aba do herdr; seção no HUD do OpenCode e no painel do Claude Code com as
  travas do repositório atual.
- Publicação no crates.io (nome livre a confirmar na hora).
