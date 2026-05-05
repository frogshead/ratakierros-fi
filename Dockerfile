FROM denoland/deno:latest
WORKDIR /app
ARG GIT_COMMIT=unknown
ENV APP_VERSION=$GIT_COMMIT
COPY ratakierros-fi.ts .
COPY public/ public/
EXPOSE 8000
CMD ["deno", "run", "--allow-net", "--allow-read", "--allow-env", "ratakierros-fi.ts"]
