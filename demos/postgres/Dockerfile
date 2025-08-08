FROM postgres:17 AS builder

RUN apt-get update && apt-get install -y \
    build-essential \
    libpq-dev \
    postgresql-server-dev-17

WORKDIR /build

COPY lttle_pg.c .
COPY Makefile .
RUN make clean
RUN make

FROM postgres:17
COPY --from=builder /build/lttle_pg.so /etc/lttle/lttle_pg.so
CMD ["postgres", "-c", "shared_preload_libraries=/etc/lttle/lttle_pg.so"]