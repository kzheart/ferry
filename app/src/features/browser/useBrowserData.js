import { useEffect, useRef, useState } from "react";
import { rpc } from "../../api/transport/rpc.js";

export function useBrowserData() {
  const [env, setEnv] = useState(null);
  const [scan, setScan] = useState(null);
  const [scanning, setScanning] = useState(false);
  const [lastScan, setLastScan] = useState(null);
  const [historyRows, setHistoryRows] = useState([]);
  const [snapRows, setSnapRows] = useState([]);
  const [pricing, setPricing] = useState(null);
  const booted = useRef(false);

  const doScan = async () => {
    if (scanning) return;
    setScanning(true);
    try { setScan(await rpc("scan")); setLastScan(Date.now()); }
    catch (error) { setScan(current => ({ tools: {}, sessions: [], error: error.message, ...(current || {}) })); }
    setScanning(false);
  };
  const loadHistory = () => rpc("history").then(setHistoryRows).catch(() => {});
  const loadSnaps = () => rpc("snapshots").then(setSnapRows).catch(() => {});
  const loadPricing = () => rpc("pricing").then(setPricing).catch(() => {});

  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    rpc("env").then(setEnv).catch(() => {});
    doScan();
    loadHistory();
    loadSnaps();
    loadPricing();
  }, []);

  return { env, scan, scanning, lastScan, historyRows, snapRows, pricing,
    doScan, loadHistory, loadSnaps };
}
