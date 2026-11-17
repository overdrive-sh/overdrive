# Cache VM: one Garage node on its own data disk. Publishes S3 credentials on
# the bus, serves until the host touches $BUS/stop.
G=/mnt/garage
mkdir -p "$G/meta" "$G/data"
SECRET="$(printf kache-spike | sha256sum | cut -c1-64)"
cat > "$G/garage.toml" <<CONF
metadata_dir = "$G/meta"
data_dir = "$G/data"
db_engine = "lmdb"
replication_factor = 1
rpc_bind_addr = "[::]:3901"
rpc_public_addr = "$SPIKE_IP:3901"
rpc_secret = "$SECRET"

[s3_api]
s3_region = "garage"
api_bind_addr = "[::]:3900"
root_domain = ".s3.garage"

[admin]
api_bind_addr = "127.0.0.1:3903"
CONF
gx() { "$GARAGE" -c "$G/garage.toml" "$@"; }
"$GARAGE" --version
gx server > "$ME/garage-server.log" 2>&1 &
gpid=$!
for _ in $(seq 60); do gx status >/dev/null 2>&1 && break; sleep 1; done
gx status
if [ ! -f "$G/creds.env" ]; then
  id="$(gx node id -q | cut -d@ -f1)"
  gx layout assign -z dc1 -c 50G "$id"
  gx layout apply --version 1
  gx bucket create kache
  gx bucket create sccache
  out="$(gx key create kache-key)"; echo "$out"
  kid="$(echo "$out" | awk -F': *' '/Key ID/{print $2}' | tr -d ' ')"
  sec="$(echo "$out" | awk -F': *' '/Secret key/{print $2}' | tr -d ' ')"
  gx bucket allow --read --write --owner kache --key kache-key
  gx bucket allow --read --write --owner sccache --key kache-key
  printf 'KACHE_S3_ACCESS_KEY=%s\nKACHE_S3_SECRET_KEY=%s\n' "$kid" "$sec" > "$G/creds.env"
fi
{ cat "$G/creds.env"; echo "KACHE_S3_ENDPOINT=http://$SPIKE_IP:3900"; echo KACHE_S3_REGION=garage; echo KACHE_S3_BUCKET=kache; } > "$ME/creds.env"
touch "$ME/ready"
glog "garage ready"
n=0
while [ ! -e "$BUS/stop" ]; do
  sleep 2; n=$((n+1))
  if [ $((n % 10)) = 0 ]; then
    { date -u +%FT%T.%3NZ; gx bucket info kache 2>&1 | grep -iE 'objects|size'; gx bucket info sccache 2>&1 | grep -iE 'objects|size'
      du -sb "$G/data" "$G/meta" 2>&1; } >> "$ME/bucket-stats.log"
  fi
done
gx bucket info kache > "$ME/bucket-info.txt" 2>&1
du -sb "$G/data" "$G/meta" > "$ME/garage-du.txt"
kill "$gpid"; wait "$gpid"
true
