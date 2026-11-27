# The OCI image carries an installation archive; it is not a runnable workload.
FROM scratch
COPY mwc-scratch-runtime-linux-amd64.tar.gz /
