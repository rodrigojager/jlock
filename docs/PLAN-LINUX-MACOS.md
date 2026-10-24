# jlock em Linux e macOS — plano

Complemento de `docs/PLAN.md` (que continua sendo a fonte de verdade do protocolo). Este plano cobre só o que
falta para o jlock ser tão confiável em Linux e macOS quanto é no Windows, e como empacotá-lo lá.

Data: 24/10/2026. Estado: proposta, para revisão antes de virar tickets.

---

## 1. Situação de partida

O código já tem os ramos `cfg(unix)`: `flock(LOCK_EX | LOCK_NB)` em `oslock.rs`, `SIGINT`/`SIGTERM`/`SIGHUP`
em `wait.rs`, `gethostname` em `holder.rs`, caminhos `$XDG_STATE_HOME/jlock` e
`~/Library/Application Support/jlock` em `home.rs`, caixa preservada no Linux e dobrada no macOS em `key.rs`,
código `128 + sinal` quando o comando do `run` morre por sinal, e os testes com `kill(pid, SIGINT)`. Os testes
unitários de normalização para Linux e macOS rodam em qualquer sistema. O `ci.yml` tem a matriz com
`ubuntu-latest` e `macos-latest`.

O que nunca aconteceu: **compilar e rodar a suíte num Linux ou macOS de verdade.** Tudo o que se segue parte
daí: o primeiro resultado útil é a primeira execução da matriz.

---

## 2. Objetivo e critério de pronto

- `cargo test` verde em `ubuntu-latest` (x86_64) e `macos-latest` (arm64) a cada push, com o teste de estresse em
  200 ciclos no CI (1000 localmente).
- O mesmo comportamento observável nos três sistemas: mesmos códigos de saída, mesmo `--json`, mesmo layout em
  disco (um `JLOCK_HOME` copiado de um Linux para um macOS continua legível).
- Um usuário de Linux ou macOS instala com um comando (`install.sh`, Homebrew, `.deb`/`.rpm`, AUR) e
  `jlock doctor` diz se a pasta onde o registro vai viver serve.
- As diferenças que não dá para apagar (sistemas de arquivos de rede, compartilhamento entre usuários, caixa de
  letras) estão documentadas e detectadas pelo `doctor`.

---

## 3. Riscos e diferenças conhecidas

| Tema | O que acontece em Unix | O que fazer |
|---|---|---|
| **Nunca executado** | Pode haver erro de compilação em ramo `cfg(unix)` que o Windows não vê, ou teste com suposição do Windows (`\`, `.exe`, `CREATE_NEW_PROCESS_GROUP`) | Rodar a matriz com `--no-fail-fast`, guardar logs como artefato, corrigir um a um (U1) |
| **`SIGPIPE` / `EPIPE`** | O Rust ignora `SIGPIPE`; `println!` entra em pânico com `Broken pipe` quando o leitor fecha (`jlock list \| head -3`, `jlock list --watch \| grep`) | Tratar `BrokenPipe` como fim normal da saída (código 0) nos caminhos de impressão; não restaurar `SIG_DFL` globalmente, porque mataria um `hold` cujo leitor de stdout sumiu (U2) |
| **`signal()` vs `sigaction()`** | `libc::signal` tem semântica que varia entre libcs; sem `SA_RESTART` uma leitura de stdin pode voltar `EINTR` | Trocar por `sigaction` com `SA_RESTART`; manter o handler só gravando num atômico (U2) |
| **Sinais e o filho do `run`** | `CTRL+C` no terminal chega ao jlock e ao filho (mesmo grupo); `kill -TERM <jlock>` chega só ao jlock, e hoje o jlock libera a trava enquanto o filho continua | No `run`, ao receber `SIGTERM`/`SIGINT`, repassar ao filho, esperar até um prazo (`JLOCK_KILL_GRACE_MS`, 5000) e então `SIGKILL`; só depois liberar (U2) |
| **Heartbeat é só do dono** | `set_modified(agora)` passa um tempo explícito: `utimensat` exige ser o dono do arquivo (ou `CAP_FOWNER`), não basta permissão de escrita | Já está correto: só o dono renova. Quem expira só renomeia e apaga, que dependem da permissão da **pasta**. Documentar (U3) |
| **`JLOCK_HOME` compartilhado entre usuários** | O padrão é por usuário (XDG), sem problema. Para dois usuários coordenarem, a pasta precisa ser gravável pelo grupo com `setgid` (2775) e **sem sticky bit** (`1777` impede renomear/apagar arquivos alheios, logo impede quebrar trava morta de outro usuário) | `doctor`: detectar sticky bit, pasta não gravável pelo grupo, `umask` que gera arquivos 0600; documentar a receita (`chmod 2775`, `umask 002`) (U3) |
| **Sistemas de arquivos de rede** | `O_EXCL` é atômico em NFSv3+ e SMB, não em NFSv2; `flock` em NFS é emulado por travas `fcntl` (Linux ≥ 2.6.12) ou devolve `ENOLCK`/`EOPNOTSUPP`; em SMB e 9p (WSL `/mnt/c`) costuma não existir; `hard_link` não existe em FAT/exFAT | `doctor` lê o tipo do sistema de arquivos (`statfs` → `f_type` no Linux, `f_fstypename` no macOS) e avisa: rede = "funciona, mas sem trava do SO; só heartbeat"; 9p/FAT = "não recomendado". `Unsupported` já é tratado (U4) |
| **WSL** | O mesmo `/mnt/c/...` visto do Windows (`c:/...`) e do Linux (`/mnt/c/...`) dá chaves diferentes; o 9p não garante `O_EXCL` | Documentar: um `JLOCK_HOME` por lado da fronteira; `doctor` avisa quando `JLOCK_HOME` está em `/mnt/<letra>` dentro do WSL (U4) |
| **Restauração em `break_file`** | Quando o rename venceu mas o arquivo estava vivo, ele volta por `hard_link`; sem hardlink (FAT/exFAT, alguns FUSE) a trava viva é quebrada | Em Linux, tentar `renameat2(RENAME_NOREPLACE)` antes do `hard_link`; caso contrário, aceitar e documentar (raro: exige corrida + sistema de arquivos sem hardlink) (U4) |
| **Symlinks e `/private/tmp`** | O jlock não canonicaliza (o caminho não precisa existir): `/tmp/x` e `/private/tmp/x` são chaves diferentes; idem qualquer symlink | Decidir: canonicalizar o maior prefixo existente (`fs::canonicalize` do ancestral) custa um `stat` por componente e muda a chave quando o caminho passa a existir. Proposta: opt-in `JLOCK_CANONICAL=1`, padrão desligado, documentado (U5) |
| **Caixa no macOS** | APFS é insensível por padrão, mas volumes sensíveis existem; o jlock dobra para minúsculas sempre → `A.ts` e `a.ts` num volume sensível caem na mesma chave (conflitam a mais, nunca a menos) | Aceitar o conservador como padrão; opt-in `JLOCK_CASE=sensitive`. Linux fica sensível (U5) |
| **Granularidade do mtime** | FUSE e alguns FS de rede têm mtime em segundos | Irrelevante com `HEARTBEAT_MS` ≥ 1000; `doctor` avisa se alguém configurou menos (U4) |
| **Terminal** | `list --watch` usa `ESC[2J ESC[H`; `enable_ansi` é vazio no Unix | Nada a fazer; testar que não suja quando stdout não é TTY (U2) |
| **Runners mais lentos** | O teste de estresse leva ~2 min no Windows local; macOS no CI é mais lento | `JLOCK_TEST_CYCLES=200` no CI (ticket 14); tempos dos testes já vêm das variáveis `JLOCK_*` (U1) |
| **Binário** | Linux com glibc dinâmica prende o binário à versão da distro | `x86_64-unknown-linux-musl` e `aarch64-unknown-linux-musl`, estáticos; macOS em dois binários (Intel e arm64) (U6) |
| **Containers** | Um `JLOCK_HOME` num volume compartilhado entre containers do mesmo kernel funciona (`O_EXCL`, `rename` e `flock` são do kernel) | Documentar como caso de uso; nada a fazer no código (U4) |

---

## 4. Empacotamento

| Canal | Como | Observações |
|---|---|---|
| `install.sh` | ticket 14 | detecta `uname -s`/`uname -m`; `~/.local/bin` por padrão |
| Homebrew | tap `rodrigojager/homebrew-tap` com `Formula/jlock.rb` apontando para os `.tar.gz` do Release; o `release.yml` abre um PR no tap com a versão e as somas | `brew install rodrigojager/tap/jlock` |
| `.deb` / `.rpm` | `cargo-deb` e `cargo-generate-rpm` **só no CI** (não entram nas dependências do produto), a partir do binário musl; anexados ao Release | metadados em `[package.metadata.deb]`/`[package.metadata.generate-rpm]` |
| AUR | `jlock-bin` com `PKGBUILD` que baixa o `.tar.gz` do Release; atualização por `git push` no repositório do AUR a partir do `release.yml` | precisa de uma chave SSH do AUR como secret |
| `cargo binstall` | ticket 14 | já cobre Unix |
| nixpkgs | depois, por contribuição | — |
| Completions | `jlock completions bash\|zsh\|fish\|powershell` gerado da mesma tabela de comandos/opções do `help` (ticket 12); pacotes instalam em `share/bash-completion/completions`, `share/zsh/site-functions`, `share/fish/vendor_completions.d` | sem `clap`: gerador próprio, ~100 linhas |
| Man page | `docs/man/jlock.1` gerado da mesma tabela por `jlock help --man` e commitado; pacotes instalam em `share/man/man1` | um teste confere que o arquivo commitado bate com o gerado |

---

## 5. Tickets propostos (`.scratch/jlock/issues/U1..U8`, quando aprovados)

| # | Entrega | Bloqueado por | Critério de pronto |
|---|---|---|---|
| U1 | Suíte verde em Linux e macOS | 14 (CI com `--no-fail-fast` e `JLOCK_TEST_CYCLES`) | matriz verde três vezes seguidas; cada correção com teste que falhava antes; `docs/DEVIATIONS.md` se algo do plano não valer em Unix |
| U2 | Sinais e pipes | U1 | `jlock list \| head -1` sai 0 sem pânico; `kill -TERM` num `run` repassa ao filho e libera depois; `sigaction` com `SA_RESTART`; testes com filhos nossos |
| U3 | Multiusuário e permissões | 13 (`doctor`) | `doctor` avisa sticky bit, pasta sem escrita de grupo e `umask` restritiva; `docs/USAGE.md` com a receita; teste unitário da lógica de checagem (dois UIDs não dá no CI) |
| U4 | Sistemas de arquivos | 13 | `doctor` mostra o tipo do FS e avisa rede/9p/FAT; `renameat2(RENAME_NOREPLACE)` no Linux em `break_file`; aviso de WSL; tabela de garantias por FS em `docs/USAGE.md` |
| U5 | Normalização Unix | — | `JLOCK_CANONICAL` e `JLOCK_CASE` opt-in, com testes unitários por plataforma e documentação do padrão conservador |
| U6 | Builds estáticos e arm64 | 14 | musl x64/arm64 e macOS x64/arm64 no Release; `file` confere "statically linked"; tamanho < 2 MB; partida < 20 ms medida no CI |
| U7 | Homebrew, `.deb`/`.rpm`, AUR | U6 | `brew install` numa VM limpa; `.deb` instala e `jlock doctor` passa; PKGBUILD validado com `namcap` |
| U8 | Completions e man page | 12 | completions para os quatro shells geradas da tabela do help; `jlock.1` commitado e conferido por teste; pacotes de U7 os instalam |

Ordem sugerida: U1 → U2 → U6 → U4 → U3 → U5 → U8 → U7. U1 e U6 são os que destravam usuários reais; U7 depende
de haver releases.

---

## 6. Decisões em aberto (para você)

1. Canonicalização de symlinks: opt-in (proposta) ou padrão ligado?
2. Caixa no macOS: dobrar sempre (proposta, conservador) ou detectar o volume?
3. `run` e sinais: repassar `SIGTERM` ao filho e esperar (proposta) ou manter o comportamento atual (liberar e sair)?
4. Pacotes `.deb`/`.rpm` já no 0.2 ou só Homebrew + `install.sh` primeiro?
