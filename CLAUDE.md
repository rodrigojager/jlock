# jlock — regras para quem trabalha neste repositório

Este repositório é o CLI `jlock` em Rust. O plano completo está em `docs/PLAN.md`; é a fonte de verdade para
formato em disco, protocolo, comandos, códigos de saída e testes. Em dúvida, o plano manda.

## Como executar os tickets

Os tickets estão em `.scratch/jlock/issues/NN-slug.md`, um por arquivo, com `**Blocked by:**`, `**Status:**` e
critérios `- [ ]`. Trabalhe a fronteira: o ticket de menor número cujos bloqueios estão todos `done`.

Para cada ticket:

1. Mude `**Status:** ready-for-agent` para `**Status:** in-progress` no arquivo do ticket.
2. Implemente só o que o ticket pede, dentro dos arquivos que ele lista em `**Files:**` (crie os que faltarem).
3. Rode `cargo fmt`, `cargo clippy --all-targets -- -D warnings` e `cargo test`. Tudo verde antes de seguir.
4. Marque cada critério cumprido (`- [x]`). Um critério que não dá para cumprir fica `- [ ]` com uma linha
   `**Note:**` explicando; não invente que passou.
5. Mude para `**Status:** done` e faça um commit só deste ticket: `NN: <título>`.
6. Diga em uma linha o que foi entregue e qual é o próximo ticket.

Quando todos estiverem `done`: rode a verificação final (`cargo test`, `cargo build --release`,
`cargo install --path .`, e `jlock --version` numa shell nova) e relate a saída literal.

## Regras

- Rust estável, edição 2021. Dependências mínimas: `serde` + `serde_json`, `sha1`, e para o SO `windows-sys`
  (Windows) / `libc` ou `rustix` (Unix). Nada de frameworks de CLI pesados; parsing de argumentos à mão ou `lexopt`.
- Sem Bun, Node ou scripts externos no produto. Os scripts em `docs/evidence/` são só evidência histórica.
- Tudo que grava no disco segue a seção 7.5 do plano: `create_new` para criar, `rename` para quebrar e para
  substituir derivados, tentativas com espera crescente em `EBUSY`/`EPERM`/`EACCES`.
- Os testes usam um `JLOCK_HOME` temporário por teste (nunca o real) e tempos curtos por variáveis de ambiente.
- Processos: os testes só disparam filhos do próprio binário de teste e só encerram esses filhos, por PID.
  Nunca encerrar processos por nome. Nunca tocar em processos ou pastas fora deste repositório.
- Nunca editar arquivos fora de `D:\jlock`, exceto `~\.cargo\bin` pelo `cargo install` no último ticket.
- Não usar modelo (IA) em nada do produto; tudo é determinístico.
- Mensagens de erro para o usuário em inglês, curtas, com o código do SO quando houver.
- Se algo do plano se mostrar inviável, não contorne em silêncio: registre em `docs/DEVIATIONS.md` (o quê, por
  quê, o que foi feito no lugar) e siga.
