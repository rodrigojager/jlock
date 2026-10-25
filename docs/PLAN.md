# jlock — plano completo

Serviço de travas por chave (arquivo, pasta ou nome) para a máquina inteira, como um CLI em Rust, sem servidor.
Um dono por chave; fila em ordem de chegada; dois pedidos simultâneos decididos por uma operação atômica do
sistema de arquivos; travas de processo morto recuperadas por heartbeat; deadlock detectado; lista de tudo que está
travado e por quem; `unlock --force`; zero arquivos sobrando quando nada está travado.

Primeiro usuário: o goal-tasks (plano em `D:\tools\goal-tasks\plano.html`), que coordena agentes editando o mesmo
checkout em paralelo. Mas o jlock não sabe o que é tarefa nem execução: serve para scripts, builds, deploys e para
qualquer agente (Claude Code, OpenCode, Codex, pi) por shell.

Data do plano: 23/10/2026. Revisão 3 (Rust, identidade genérica, zero sobras, nota multiplataforma).
Revisão 4, 24/10/2026: o plano absorve o que a implementação decidiu onde ele era omisso ou foi superado pelo
ticket 09 (palavra solta é caminho; pedido repetido do mesmo dono; código 130; espera por hierarquia com uma senha
só; tentativas curtas em derivados; trava do SO conta como vida). Os pontos estão marcados com **[rev. 4]**; o
histórico de cada um está em `docs/DEVIATIONS.md`.
Revisão 5, 24/10/2026: a fase 2 da seção 14 (idiomas, help didático, `about`, `doctor`, releases, instaladores,
`update`, READMEs) deixa de ser proposta e passa a descrever o que foi implementado; comandos e variáveis novos
entram nas tabelas da seção 6. Os pontos estão marcados com **[rev. 5]**.
Revisão 6, 25/10/2026: `jlock renew` (ticket 19) estende o prazo (`ttl`) de uma trava viva sem soltá-la; o prazo
novo fica em `lease.json`, ao lado de `holder.json`. Os pontos estão marcados com **[rev. 6]**.

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
│     ├─ lease.json                  ← [rev. 6] prazo dado por `renew`; só vale para a concessão que nomeia
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
  mas descontrolado); expira com registro, como a morte. **[rev. 6]** O dono pode adiar esse prazo com
  `jlock renew` (seção 7, "renew"): o prazo efetivo passa a ser o `until` de `lease.json`.

### Corpo de `lease.json` **[rev. 6]**

```json
{ "owner": "claude:a8ec2aca", "since": "2026-10-23T23:06:50.123Z", "until": "2026-10-24T03:06:50.123Z" }
```

- `owner` e `since` são os do `holder.json` da concessão que ele estende. Só vale enquanto os dois batem: um
  `lease.json` que sobrou de outra concessão (mesmo do mesmo dono) é ignorado e sai junto com a pasta da chave.
- Gravado por `.tmp` + rename (`lease.<pid>-<rand>.tmp` na pasta da chave): quem lê vê o antigo ou o novo.
  Ilegível conta como ausente, e o prazo volta a ser `since + ttl`.
- `pid`, `host`, `cwd`, `since` preenchidos pelo jlock. **[rev. 4]** O `pid` é só informação: o jlock não confere
  se o processo existe (PID pode ser reutilizado). A segunda pista de vida é a trava do SO sobre `holder.json`,
  que `hold` e `run` mantêm (seção 7, "Trava do SO").

### Linha do `log.jsonl`

```json
{ "at": "2026-10-23T23:06:50.123Z", "event": "acquire|release|renew|queued|expired|forced|deadlock|lost|gc",
  "key": "…", "owner": "…", "group": "…", "by": "…", "reason": "…", "waited_ms": 1234 }
```

---

## 5. Normalização de chaves

- Entrada que parece caminho (contém `/`, `\`, ou começa com letra de unidade, `.`, `~`): resolver para absoluto a
  partir do cwd; `~` → home; separadores → `/`; em Windows e macOS, minúsculas (sistemas insensíveis a caixa); em
  Linux, manter caixa; remover barra final; `\\?\` removido. Não exige que o caminho exista.
- Entrada `name:<texto>`: chave literal, sem normalização além de aparar espaços.
- `*`: chave universal.
- **[rev. 4]** Qualquer outra entrada (uma palavra solta como `Cargo.toml` ou `build`) é caminho, relativo ao cwd.
  Agentes passam nomes de arquivo sem `./`; tratá-los como nome faria dois agentes em pastas diferentes não
  conflitarem em silêncio. Para um nome, escreva `name:`.
- Hash: sha1 da chave normalizada, em hex, nome do diretório. O arquivo `key` guarda o texto.
- Conflito hierárquico (só chaves-caminho): `a` conflita com `b` se `a == b`, `a` é prefixo de diretório de `b`
  (`a + "/"` prefixo de `b`) ou vice-versa. `*` conflita com tudo. `name:` só com igual.

---

## 6. O CLI

| Comando | O que faz |
|---|---|
| `jlock acquire <chave…> [--owner id] [--group g] [--intent "…"] [--meta k=v…] [--ttl 2h] [--wait\|--no-wait] [--timeout 30s] [--json]` | pega todas as chaves (ordem canônica, tudo ou nada). Padrão: espera sem limite. Saída: `granted`; `queued` (dono e posição) com `--no-wait`; `timeout` (dono e posição) com `--timeout` vencido; `deadlock` com a cadeia. **[rev. 4]** Na linha de texto, `--no-wait` escreve `busy <chave> held by <dono> (position N)`; o `--json` diz `queued`. Pedir de novo uma chave que o mesmo `owner` já tem é `granted` na hora. |
| `jlock run <chave…> [opções de acquire] -- <comando…>` | como `flock(1)`: pega, mantém heartbeat, roda o comando, libera ao terminar (ou ao morrer: expira). Código de saída do comando é repassado. |
| `jlock hold <chave…> [opções de acquire] [--detach \| --release-on-eof]` | fica vivo renovando o heartbeat; encerra ao receber a linha `release` na entrada padrão, SIGINT/CTRL+C, ou ao ser encerrado. Imprime `granted` e depois eventos (`lost`). **[rev. 5]** Quando o próprio dono devolve uma chave com `jlock release` (o log diz `release` dele), o `hold` imprime `released`, segue com as outras e sai com 0 quando não sobra nenhuma; `lost`/14 fica para trava quebrada ou expirada. Com `--detach`, responde como o `hold` (mesma saída, mesmo código) e volta, enquanto um segundo `jlock hold`, desligado do terminal, mantém a trava até `release`, `--ttl` ou `unlock --force`: uma chamada só para um agente. Com `--release-on-eof` (ticket 18, 24/10/2026), o fim da entrada padrão vale como a linha `release`: um programa que fica aberto segurando o `hold` em primeiro plano solta a trava no instante em que morre; não combina com `--detach`. |
| `jlock release [<chave…>] [--owner id] [--group g]` | libera (só o dono). Sem chaves: tudo do dono; com `--group`: tudo do grupo. |
| `jlock renew [<chave…>] [--owner id] [--group g] [--ttl 2h] [--json]` | **[rev. 6]** adia o prazo das travas do dono para agora + ttl (sem `--ttl`, o ttl da concessão) e renova o heartbeat, sem soltar a trava. Sem chaves: todas do dono (do grupo, com `--group`). Saída: `renewed <chave> until <hora>`; 13 para chave de outro; 14 para trava que já venceu. |
| `jlock heartbeat <chave> --owner id` | renova; `lost` se a trava não é mais sua. (Para quem não usa `hold`.) |
| `jlock status <chave>` | dono e fila da chave, inclusive conflitos por hierarquia (pasta acima ou arquivos dentro). |
| `jlock list [--owner x] [--group g] [--under D:\proj] [--stale] [--json] [--watch]` | tudo que está travado na máquina: chave, dono, grupo, intenção, há quanto tempo, heartbeat, fila. `--watch` redesenha a cada segundo. |
| `jlock graph [--json]` | quem espera por quem; ciclos. |
| `jlock unlock (<chave> \| --owner x \| --group g) --force [--reason "…"]` | quebra travas de outrem, registrando quem forçou; o dono recebe `lost` no próximo heartbeat. |
| `jlock gc [--json]` | varredura global agora: expira travas e senhas mortas, consome `broken\`, remove pastas de chave vazias, rotaciona o log, reescreve o índice; imprime o que removeu. |
| `jlock doctor [--json]` | **[rev. 5]** diagnostica sem consertar: versão e executável, `JLOCK_HOME` (origem, existe, é pasta, gravável), as primitivas `create_new`/`rename`/`remove` nele, a trava do SO, o sistema de arquivos, as variáveis `JLOCK_*`, outro `jlock` antes no `PATH`, o estado do registro (sugere `gc` para sobras) e o `curl`. Cada `warn`/`fail` traz a correção. Saída 0 sem `fail` (avisos não mudam o código), 20 com `fail`. Não cria `JLOCK_HOME`, não roda `gc`, não mexe no `PATH`; seus temporários são apagados mesmo em falha. |
| `jlock update [--check] [--version vX.Y.Z] [--json]` | **[rev. 5]** último Release (ou a versão pedida) → baixa o arquivo do alvo atual → confere com o `SHA256SUMS` do Release → troca o próprio executável (Windows: o antigo vira `jlock.exe.old`, apagado pelo próximo jlock) → roda o novo com `--version` e, se não bater, repõe o antigo. `--check` só compara (código 0). Usa o `curl` do sistema (PowerShell no Windows como reserva); sem nenhum, código 20. **O jlock só acessa a rede em `update`, e só quando chamado.** Sem release publicado: código 20, "no release found". |
| `jlock help [<comando>]`, `jlock <comando> --help` | **[rev. 5]** ajuda geral agrupada por intenção (pegar e soltar, ver, emergência, manutenção, ajuda); por comando: sinopse, descrição, uma linha por opção (o que faz, padrão, exemplo), exemplos, códigos de saída, veja também. Gerada da mesma tabela de comandos e opções que o despacho usa para recusar opções desconhecidas (código 2, com sugestão do nome mais parecido). Quebrada em 100 colunas. |
| `jlock about [--json]` | **[rev. 5]** nome, versão, descrição, autor (Rodrigo Jäger), GitHub `https://github.com/rodrigojager`, LinkedIn `https://www.linkedin.com/in/rodrigojager`, repositório, licença, links da documentação; sem rede |
| `jlock --version`, `jlock --help` | `--version` é uma linha só (`jlock 0.1.0`), estável para scripts; o rodapé do `--help` traz o autor e os links |
| `--lang <tag>` (global, em qualquer posição antes de `--`) | **[rev. 5]** idioma das saídas de texto desta execução: `en`, `pt-BR`, `es`, `fr`, `de`, `it`, `zh-CN`, `ja`, `ru`, `ar`, `hi`. Prioridade: `--lang` > `JLOCK_LANG` > idioma do sistema > `en`. Comandos, opções, `--json`, `log.jsonl`, `index.json` e códigos de saída não mudam com o idioma. |

### Códigos de saída

| Código | Significado |
|---|---|
| 0 | concedido / ok |
| 10 | ocupado (ficou na fila; só com `--no-wait`) |
| 11 | timeout esperando |
| 12 | deadlock |
| 13 | não é o dono (release/heartbeat/renew) |
| 14 | trava perdida (`hold`/`run` terminam com 14 se o dono descobriu `lost`); **[rev. 6]** `renew` de uma trava que já passou do prazo |
| 2 | uso incorreto (argumentos) |
| 20 | erro de sistema de arquivos (permissão, disco cheio, arquivo preso); a mensagem traz o código do SO e o caminho |
| 130 | **[rev. 4]** interrompido (CTRL+C/SIGINT/SIGTERM) durante a espera; a senha foi apagada antes de sair |

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
| `JLOCK_LANG` | idioma do sistema, senão `en` | **[rev. 5]** idioma das saídas de texto (tag exata, depois a língua primária: `pt` → `pt-BR`); tag desconhecida → aviso e inglês |
| `JLOCK_HINTS` | `1` | **[rev. 5]** `0` desliga as dicas (`tip: ...`, no stderr) que algumas saídas de texto trazem; nunca aparecem no `--json` |
| `JLOCK_UPDATE_BASE` | GitHub do projeto | **[rev. 5]** origem alternativa para `update`: `<base>/releases/latest` e `<base>/releases/download/<tag>/`; aceita `file://` (os testes usam uma pasta) |

Os testes usam valores curtos por essas variáveis.

---

## 7. O protocolo

### acquire(key, owner)

```
(0) [rev. 4] holder.json já é do owner → granted na hora, sem tocar no arquivo (intent/meta/ttl do novo pedido
    são ignorados). Travas e senhas do próprio owner nunca o bloqueiam, nem pela hierarquia.
(1) hierarquia [rev. 4]: a senha é uma só, na própria chave. Em cada sondagem, além da própria fila, o esperador
    lê as chaves conflitantes (ancestrais, descendentes, `*`) e só segue quando nenhuma delas tem holder.json de
    outro dono nem senha de outro dono mais antiga que a sua. Como o nome da senha é a hora de chegada, a ordem
    entre filas é a ordem de chegada. Logo depois do create_new, o novo dono relê as chaves conflitantes: se
    outra foi tomada no mesmo instante por quem ainda não o via, apaga o próprio holder.json e volta a esperar;
    a senha mais antiga vence na sondagem seguinte. (Por que não uma senha em cada chave conflitante: criar,
    renovar e apagar várias juntas deixa estado parcial em chaves alheias se o processo morrer, e serializaria
    `a.ts` e `b.ts` pela fila de `src` mesmo sem conflitarem entre si.)
(2) deadlock: grafo = (senhas → donos das chaves) + aresta (owner → dono da chave); ciclo contendo owner → recusar
    { deadlock, chain }
(3) garantir locks/<hash>/ e queue/ ; gravar `key` se não existir
(4) senha ← criar queue/<agora-µs>-<4 hex aleatórios>.json com create_new (conteúdo: corpo da seção 4)
    ENOENT ao criar (a pasta foi apagada por um release neste instante) → voltar a (3)
    [rev. 4] com --no-wait não há senha: chave livre, sem fila e sem conflito → criar holder.json; senão
    queued { holder, position } com posição = tamanho da fila + 1
(5) laço, a cada POLL_MS com jitter:
      renovar mtime da minha senha
      descartar senhas com heartbeat > STALE_MS (renomear para broken/: só um consegue; registrar)
      holder.json com heartbeat > STALE_MS, ou com ttl vencido → renomear para broken/ (só um consegue),
          registrar "expired", apagar o arquivo em broken/
          [rev. 4] antes de quebrar por heartbeat, tentar a trava do SO em holder.json: ocupada → o dono está
          vivo (hold/run); não quebrar. O ttl vence mesmo com a trava do SO.
      se minha senha é a menor viva, holder.json não existe e nenhuma chave conflitante bloqueia (passo 1):
          criar holder.json com create_new: conseguiu → apagar minha senha → granted
                                             EEXIST → outro passou à frente neste instante; registrar; continuar
      senão → timeout vencido → apagar minha senha → timeout { holder, position }
      [rev. 4] CTRL+C/SIGINT/SIGTERM → apagar minha senha → sair com 130
      a cada 8 sondagens: refazer o teste de deadlock; ciclo e minha senha é a mais nova → apagar senha, deadlock
```

Para várias chaves: ordenar as chaves normalizadas; adquirir uma a uma; se alguma falhar (timeout/deadlock),
liberar as já obtidas e devolver qual faltou.

### heartbeat(key, owner)

Ler `holder.json`; `owner` diferente ou arquivo ausente → `lost`; senão renovar mtime (`set_file_times` para agora).

### renew(key, owner, ttl) **[rev. 6]**

Ler `holder.json`; `owner` diferente ou arquivo ausente → `not_owner` (13). Prazo efetivo já vencido → expirar
como na varredura (`expired`, motivo `ttl`), registrar `lost` e responder 14: o que venceu não ressuscita. Senão,
com `ttl` (o pedido, ou o `ttl_secs` da concessão), gravar `lease.json` com `until = agora + ttl` por `.tmp` +
rename; renovar o mtime de `holder.json`; ler `holder.json` de novo e, se a concessão mudou nesse meio-tempo
(outro `owner` ou outro `since`), responder `not_owner` (o `lease.json` nomeia a concessão antiga, então não
vale para ninguém). Registrar `renew` com `until`; quem procura no log se a trava foi devolvida (o `hold`) pula
os `renew`, que não mudam a posse. A pasta sumiu no meio (um `release` ganhou) → `not_owner`, sem erro; sem chaves,
uma chave com erro de disco não impede as outras (o erro sai no fim, código 20). `holder.json` nunca é reescrito: a trava do SO de quem a
mantém (`hold`/`run`) e o heartbeat seguem intactos, e a chave não fica livre em instante nenhum. Sem chaves,
renova todas as travas do dono (só as do grupo, com `--group`).

Prazo efetivo de uma trava, usado por `hold`, `run`, `heartbeat`, expiração, `status`, `list` e `doctor`: o
`until` de `lease.json` quando ele é desta concessão; senão `since + ttl`; sem nenhum dos dois, não há prazo.
Um `hold` iniciado por um jlock anterior à 0.2.0 não lê `lease.json` e termina no prazo antigo.

### release(key, owner)

`owner` diferente → `not_owner`; senão apagar `holder.json`; se `queue/` vazia e `broken/` vazia → apagar `key`,
`queue/`, `broken/` e a pasta da chave (`rmdir` falha com `ENOTEMPTY` se alguém acabou de entrar: tudo bem).

### force(key | owner | group)

Renomear `holder.json` para `broken/<ts>-forced.json`, ler, registrar `forced` com `by` e `reason`, apagar.
Senhas não são afetadas; o próximo da fila pega.

### Trava do SO (`hold`/`run`) **[rev. 4]**

`hold` e `run` mantêm `holder.json` aberto com uma trava do sistema operacional: `LockFileEx` exclusiva e sem
espera no Windows (num byte alto, fora do conteúdo; handle com compartilhamento de leitura, escrita e exclusão, para
não impedir `list` nem `rename`) e `flock(LOCK_EX | LOCK_NB)` em Unix. É tomada antes de anunciar `granted` e solta
ao liberar; o kernel a solta no instante em que o processo morre. Serve de segunda pista de vida: quem expira a
consulta antes de quebrar por heartbeat, e `list`/`status` mostram `late` (vivo, atrasado) em vez de `dead`. Não
obtê-la (sistema de arquivos sem suporte) não impede nada: fica registrado `oslock: unsupported` e o heartbeat
manda sozinho, como para travas de `acquire` puro, que nunca têm trava do SO.

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
- `index.json` e rotação do log: escrever `.tmp` e `rename` por cima. **[rev. 4]** O `index.json` é derivado e
  reescrito por todo comando que mudou algo; o rename dele faz só 4 tentativas de 10 ms e desiste (a próxima
  mudança ou varredura reescreve), para não atrasar `acquire`/`release` por um arquivo que não é a verdade.
- Tentativas em `EBUSY`/`EPERM`/`EACCES` em operações de remoção, rename, criação de pasta e leitura: 50, 100, 200,
  400, 800, 1600 ms; depois erro 20 com a mensagem do SO e o caminho. Nunca matar processos. **[rev. 4]** No
  Windows, uma pasta ou arquivo que outro processo está apagando responde `EACCES` ou "já existe" por um instante;
  por isso a leitura e a criação de pasta também entram na escada.

---

## 8. Zero sobras

| Arquivo | Nasce | Morre | Se o responsável morrer | Pior sobra |
|---|---|---|---|---|
| `locks\<hash>\lease.json` **[rev. 6]** | no `renew` | junto com a pasta da chave (só então: enquanto `holder.json` passa por `broken/`, ele ainda vale) | ignorado (vale só para a concessão que nomeia) e sobrescrito no próximo `renew`; a varredura remove `lease.*.tmp` de quem morreu | até a pasta da chave sumir |
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
| trava de processo morto recuperada em ≤ STALE_MS, com registro | **[rev. 4]** distinguir morto de congelado > STALE_MS numa trava de `acquire` puro (o congelado recebe `lost` e para). Com `hold`/`run`, a trava do SO distingue: o congelado mantém a trava até o `ttl` ou um `unlock --force` |
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
| dono congelado > STALE_MS mas vivo | **[rev. 4]** trava de `acquire` puro: quebrada; ele vê `lost` e para (código 14). `hold`/`run`: a trava do SO mostra que está vivo; a trava fica (`list` marca `late`) até o `ttl` ou um `unlock --force` |
| `unlock --force` por engano | `log.jsonl` diz quem, quando, por quê; o dono para em vez de sobrescrever |
| antivírus/editor segurando arquivo do jlock | tentativas com espera crescente; persistiu → código 20, mensagem do SO, sem matar processo |
| relógio salta | pode expirar travas vivas de `acquire` puro uma vez; donos recebem `lost` e param. **[rev. 4]** `hold`/`run` ficam protegidos pela trava do SO |
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
14. **[rev. 6]** `renew` (`tests/renew.rs`): um `hold --ttl 2s` renovado passa do prazo original sem `lost` e
    termina no novo; sem `--ttl` vale o ttl da concessão; dono errado → 13 sem `lease.json`; prazo vencido → 14
    e a chave livre; sem chaves, só as do dono (e do grupo); `lease.json` de outra concessão não segura nada;
    `locks/` vazia no fim de cada caso.

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

---

## 14. Fase 2: para qualquer pessoa usar **[rev. 5]**

Proposta de 24/10/2026, executada no mesmo dia pelos tickets 11–17 (em `.scratch/jlock/issues/`). Os comandos e
as variáveis novos estão nas tabelas da seção 6, marcados com **[rev. 5]**; o histórico das escolhas que os
tickets deixavam em aberto está em `docs/DEVIATIONS.md` (§8–§13). O roadmap público está em `ROADMAP.md`; Linux e
macOS têm plano próprio em `docs/PLAN-LINUX-MACOS.md`.

### Decisões, como implementadas

1. **Idiomas.** Inglês é a referência e a reserva para qualquer chave que falte. Catálogos JSON planos embutidos
   no binário (`locales/<tag>.json`, `include_str!`, lidos uma vez por processo), sem dependência nova além de
   `Win32_Globalization` do `windows-sys`; `t!`/`txt!` em toda string de texto, `_one`/`_other` para plural.
   Idiomas: `en`, `pt-BR`, `es`, `fr`, `de`, `it`, `zh-CN`, `ja`, `ru`, `ar`, `hi`, cada um com `_glossary`.
   Escolha: `--lang` (em qualquer posição antes de `--`) > `JLOCK_LANG` > idioma do sistema (`LC_ALL`,
   `LC_MESSAGES`, `LANG`; `GetUserDefaultLocaleName` no Windows) > `en`; tag exata, depois a língua primária;
   tag desconhecida dada pelo usuário → aviso em inglês e segue em inglês. Comandos, opções, `--json`,
   `log.jsonl`, `index.json` e códigos de saída não mudam com o idioma; as mensagens de erro no `--json` saem
   em inglês. Larguras contadas como o terminal mostra (CJK = 2 colunas). `zh-CN`, `ja`, `ru`, `ar` e `hi`
   aguardam revisão de falante nativo; revisão de textos é aceita em qualquer idioma (`CONTRIBUTING.md`).
2. **Help didático.** Uma tabela única de comandos e opções (`src/commands.rs`, DEVIATIONS §9) gera
   `jlock help [<comando>]` / `jlock <comando> --help` (sinopse, descrição, uma linha por opção com padrão e
   exemplo, exemplos, códigos de saída, veja também; quebra em 100 colunas) e é usada pelo despacho para
   recusar opções desconhecidas (código 2, com o nome mais parecido). `jlock about [--json]` com autor e
   links, de constantes únicas em `src/cmd/about.rs`. Dicas (`tip: ...`) no stderr depois de `busy`,
   `timeout`, `deadlock`, `not_owner` e `lost`, nunca no `--json`, desligáveis com `JLOCK_HINTS=0`
   (DEVIATIONS §8). `list` com rodapé de contagem e legenda de `(dead)`/`(late)`; `status` explica por que
   cada conflito bloqueia.
3. **`doctor`.** Nove checagens com `ok`/`warn`/`fail` e correção; código 0 sem `fail`, 20 com `fail`. Só
   sugere: não cria `JLOCK_HOME` (ausente é `fail`, com o comando que cria), não roda `gc`, não mexe no
   `PATH`; os temporários dele são apagados mesmo em falha.
4. **Releases e instalação em um comando.** Tag `v*` → `release.yml` confere a versão com o `Cargo.toml`, roda
   os testes e compila `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
   `x86_64-apple-darwin` (no runner arm64, DEVIATIONS §10) e `aarch64-apple-darwin`; empacota
   `jlock-<versão>-<alvo>.zip|.tar.gz` com o binário, `LICENSE` e `README.md`; gera `SHA256SUMS`; publica o
   GitHub Release. `install.sh` (POSIX sh) e `install.ps1` (PowerShell 5.1 e 7) na raiz, com verificação da
   soma; `cargo binstall jlock`. Passo a passo em `docs/RELEASING.md`.
5. **`update` pelo `curl` do sistema.** Sem cliente HTTP em Rust; PowerShell como reserva no Windows; o `tar`
   do sistema abre o arquivo (DEVIATIONS §12); SHA-256 próprio em `src/sha256.rs` (DEVIATIONS §11). Troca o
   próprio executável (no Windows o antigo vira `jlock.exe.old`, apagado pelo próximo jlock que inicia),
   confere o `--version` do novo e repõe o antigo se falhar. **O jlock só acessa a rede em `update`, e só
   quando chamado.**
6. **READMEs** nos onze idiomas, com a mesma estrutura, barra de idiomas e exemplos; as saídas dos blocos
   `<!-- jlock-example -->` são reexecutadas por `tests/readmes.rs` no idioma de cada arquivo.
7. **Caminhos: symlinks e caixa (todos os sistemas)** — decidido, ainda não implementado (U5 em
   `docs/PLAN-LINUX-MACOS.md`). Symlinks resolvidos por padrão, canonicalizando o maior prefixo existente
   (`JLOCK_CANONICAL=0` desliga): só junta nomes do mesmo arquivo numa chave, nunca separa. Caixa dobrada no
   Windows e no macOS, preservada no Linux; `JLOCK_CASE=sensitive` para volumes sensíveis no macOS. Entra na
   seção 5 quando implementado.
8. **`run` e sinais** — decidido, ainda não implementado (U2). `SIGTERM`/`SIGINT` recebido pelo `run` é
   repassado ao comando; prazo `JLOCK_KILL_GRACE_MS` (5000) antes do `SIGKILL`; só então libera e sai com
   `128 + sinal`. Hoje o `run` ignora o sinal e segura até o comando terminar.
9. **Pacotes Unix.** 0.2: `install.sh` (feito), Homebrew e `cargo binstall` (feito). `.deb`/`.rpm` e AUR só no
   0.3, depois da suíte verde em Linux de verdade (U1, U7).
10. **`hold --detach` para agentes** (pedido do autor, 24/10/2026, depois dos tickets). Um agente passa minutos
   entre dois comandos, e uma trava de `acquire` puro expira em `JLOCK_STALE_MS` sem `heartbeat`; manter um
   `hold` vivo exigia uma chamada em segundo plano e outra para saber a resposta. Com `--detach`, uma chamada
   espera a vez, responde (mesma saída e mesmo código do `hold`) e volta; um segundo `jlock hold`, iniciado
   desligado do terminal (sessão própria no Unix; `DETACHED_PROCESS` e, quando o job permite, fora do job do
   chamador no Windows) e sem herdar os handles padrão do primeiro, mantém a trava até `release`, `--ttl` ou
   `unlock --force`. Junto: o `release` do próprio dono encerra um `hold` com 0, sem registrar `lost` (o
   `hold` confere no fim do log se o último evento da chave é um `release` desse dono).
11. **`jlock renew`** **[rev. 6]** (pedido do autor, 25/10/2026, ticket 19). O jtasks segura as travas de uma
   tarefa com `--ttl` e tarefas passam de horas; o prazo vencia com o dono vivo, e pedir de novo não estendia
   nada. Devolver e pegar de novo deixava a chave livre por um instante e, com `--detach`, um processo antigo
   vivo a cada vez. `renew` adia o prazo no lugar, sem soltar a trava (seção 7, "renew").

### O que ficou para o CI ou para o autor

- `install.sh` só foi exercitado no `sh` do Git Bash; o `shellcheck` e o teste Unix de `tests/installers.rs`
  rodam no CI (Ubuntu e macOS).
- O primeiro Release depende de uma tag publicada pelo autor (`docs/RELEASING.md`); até lá, `jlock update`
  responde código 20, "no release found".

### Ordem executada

11 i18n (`en` + `pt-BR`) → 12 help/about → 13 doctor → 14 release e instaladores → 15 update → 16 traduções
para os nove idiomas restantes → 17 READMEs. Depois: Linux e macOS (`docs/PLAN-LINUX-MACOS.md`, tickets U1–U8).
