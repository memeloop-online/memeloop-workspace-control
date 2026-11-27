FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends openssh-server sudo python3 ca-certificates && rm -rf /var/lib/apt/lists/* && \
    userdel -r ubuntu && useradd -m -u 1000 -s /bin/bash user && echo 'user ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/user && chmod 0440 /etc/sudoers.d/user && \
    passwd -d user && mkdir -p /run/sshd /home/user/.ssh && chown -R user:user /home/user/.ssh
COPY workload.sh /usr/local/bin/mwc-budget-workload
RUN chmod 0755 /usr/local/bin/mwc-budget-workload && python3 -c 'open("/seed-copy-up", "wb").write(b"image-lower\n")'
ENTRYPOINT ["/usr/local/bin/mwc-budget-workload"]
