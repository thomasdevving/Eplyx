# syntax=docker/dockerfile:1
#
# The Eplyx frontend, served as static bytes.
#
# A second image rather than a second job in the API's: that image deliberately
# carries the server binary and nothing else, and adding a JavaScript runtime to
# it to serve a few hundred kilobytes of static files would trade a real
# property for a small convenience.
#
# Nothing secret is built in. `EPLYX_API_URL` is a public hostname and the only
# deployment-specific value; the credential a person uses is pasted into their
# own browser tab and never reaches this image or this process.

FROM node:22-slim AS builder
WORKDIR /build
ENV PNPM_HOME=/usr/local/bin
RUN corepack enable

# The dependency install is its own layer: `three` is the only runtime
# dependency and it moves far less often than the app does.
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

COPY frontend ./frontend
# Baked at build time into public/runtime-config.js, because the browser reads it
# before the app starts. Empty is legal and means "same origin", which fails
# loudly rather than quietly pointing at somebody's laptop.
ARG EPLYX_API_URL=""
ENV EPLYX_API_URL=${EPLYX_API_URL}
RUN node frontend/build.mjs

FROM node:22-slim AS runtime
WORKDIR /app
COPY --from=builder /build/dist ./dist
COPY --from=builder /build/frontend/serve.mjs ./frontend/serve.mjs
EXPOSE 8080
CMD ["node", "frontend/serve.mjs"]
