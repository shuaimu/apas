# Build from the repository root. Inputs are deliberately operator-supplied:
# deploy/autonomous-jobs/inputs/{apas,bun,omp-with-dependencies.tar.gz}.
# The OMP archive contains the pinned source AND its pre-resolved node_modules;
# no package manager or network installation runs in this image build.
# TOOLCHAIN_IMAGE must provide /bin/sh, Python 3, Git, tar, sha256sum and all
# approved verifier toolchains. Use an immutable registry@sha256 reference.
ARG TOOLCHAIN_IMAGE
FROM ${TOOLCHAIN_IMAGE}
ARG TOOLCHAIN_IMAGE
ARG APAS_SHA256
ARG BUN_SHA256
ARG OMP_SOURCE_SHA256
ARG OMP_VERSION=18.18.0
USER 0
COPY deploy/autonomous-jobs/inputs/apas /usr/local/bin/apas
COPY deploy/autonomous-jobs/inputs/bun /usr/local/bin/bun
COPY deploy/autonomous-jobs/inputs/omp-with-dependencies.tar.gz /tmp/omp.tar.gz
RUN case "$TOOLCHAIN_IMAGE" in *@sha256:*) ;; *) exit 1 ;; esac \
 && test "${#APAS_SHA256}" = 64 && test "${#BUN_SHA256}" = 64 && test "${#OMP_SOURCE_SHA256}" = 64 \
 && printf '%s  %s\n' "$APAS_SHA256" /usr/local/bin/apas "$BUN_SHA256" /usr/local/bin/bun "$OMP_SOURCE_SHA256" /tmp/omp.tar.gz | sha256sum --check --strict \
 && chmod 0755 /usr/local/bin/apas /usr/local/bin/bun \
 && mkdir -p /opt/omp /usr/local/lib/apas /workspace /session /run/apas-bridge \
 && tar --extract --gzip --file /tmp/omp.tar.gz --directory /opt/omp --strip-components=1 --no-same-owner \
 && rm /tmp/omp.tar.gz \
 && test -f /opt/omp/packages/coding-agent/src/main.ts \
 && test -d /opt/omp/node_modules \
 && /usr/local/bin/bun -e 'const p=require("/opt/omp/packages/coding-agent/package.json"); if(p.version!==process.argv[1])process.exit(1)' "$OMP_VERSION" \
 && /usr/bin/python3 --version && /usr/bin/git --version
COPY crates/client-cli/src/jobs/omp-worker.mjs /usr/local/lib/apas/omp-worker.mjs
COPY deploy/autonomous-jobs/apas-omp-worker /usr/local/bin/apas-omp-worker
COPY deploy/autonomous-jobs/apas-job-entrypoint /usr/local/bin/apas-job-entrypoint
RUN chmod 0755 /usr/local/bin/apas-job-entrypoint /usr/local/bin/apas-omp-worker \
 && chmod -R a-w /opt/omp /usr/local/lib/apas \
 && test -x /usr/local/bin/apas && test -x /usr/local/bin/bun
# Runtime overrides with rootless keep-id; neither worker nor verifier runs root.
USER 65532:65532
WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/apas-job-entrypoint"]
CMD ["/usr/local/bin/apas-omp-worker", "--version"]
