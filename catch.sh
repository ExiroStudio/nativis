cargo run --bin nativis > scratch/nativis_verify2.log 2>&1 &
NPID=$!
sleep 2

if ! kill -0 $NPID 2>/dev/null; then
  echo "PROSES SUDAH MATI. Isi log:"
  cat scratch/nativis_verify2.log
  exit 1
fi

echo "PID proses Nativis: $NPID"

# Cari XID lewat _NET_WM_PID, bukan lewat judul — ini tidak bisa salah tangkap
for xid in $(xprop -root _NET_CLIENT_LIST | grep -oP '0x[0-9a-f]+'); do
  wpid=$(xprop -id "$xid" _NET_WM_PID 2>/dev/null | awk '{print $3}')
  if [ "$wpid" = "$NPID" ]; then
    echo "COCOK — XID milik proses Nativis ($NPID): $xid"
    xprop -id "$xid" _NET_WM_WINDOW_TYPE
    xprop -id "$xid" _NET_WM_STATE
  fi
done

xprop -root _NET_CLIENT_LIST_STACKING
kill $NPID 2>/dev/null
