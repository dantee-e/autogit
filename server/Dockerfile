FROM python:3.13-slim
WORKDIR /app
COPY pyproject.toml .
COPY paddock_server ./paddock_server
RUN pip install --no-cache-dir .
ENV PADDOCK_DATA=/data
VOLUME /data
EXPOSE 8000
CMD ["paddock-server", "serve"]
