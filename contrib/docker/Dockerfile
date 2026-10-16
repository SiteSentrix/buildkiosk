# Container image for a single self-hosted BuildKiosk server (see docs/deployment.md).
#
# The image holds the same static (musl) binary that the release archives hold; nothing is compiled here. The
# release workflow puts the binaries in the build context as <arch>/buildkiosk (amd64 and arm64) and builds a
# multi-architecture image. To build one by hand from a release archive:
#
#   mkdir -p ctx/amd64 && tar -xzf buildkiosk-<version>-x86_64-unknown-linux-musl.tar.gz --strip-components=1 \
#     -C ctx/amd64 buildkiosk-<version>-x86_64-unknown-linux-musl/buildkiosk
#   docker build -f contrib/docker/Dockerfile ctx -t buildkiosk
#
# Runs as an unprivileged user (uid 65532) with the company data in the /data volume. The server speaks plain HTTP on
# port 8080: put a reverse proxy that terminates HTTPS in front of it and add --secure-cookies to the command.

# Only used to make an empty /data owned by the unprivileged user (the final image has no shell).
FROM --platform=$BUILDPLATFORM alpine:3.22 AS prep
RUN mkdir /data && chown 65532:65532 /data

# CA certificates (for outgoing email over TLS) and the unprivileged user come from the base image.
FROM gcr.io/distroless/static-debian12:nonroot
ARG TARGETARCH
COPY --from=prep --chown=65532:65532 /data /data
COPY --chown=root:root --chmod=0755 ${TARGETARCH}/buildkiosk /usr/local/bin/buildkiosk
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/buildkiosk"]
CMD ["serve", "--data", "/data", "--bind", "0.0.0.0:8080"]
