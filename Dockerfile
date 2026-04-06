FROM denoland/deno:latest
WORKDIR /app
COPY ratakierros-fi.ts .
COPY public/ public/
EXPOSE 8000
CMD ["deno", "run", "--allow-net", "--allow-read", "ratakierros-fi.ts"]
