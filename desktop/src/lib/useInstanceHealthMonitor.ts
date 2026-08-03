import { useCallback, useEffect, useRef, useState } from 'react';
import {
  checkAllInstanceHealth,
  type HealthReport,
} from './tauri';

const HEALTH_SCAN_INTERVAL_MS = 2 * 60 * 1000;

export interface InstanceHealthReports {
  [instanceId: string]: HealthReport;
}

export interface InstanceHealthErrors {
  [instanceId: string]: string;
}

/**
 * Keeps health state current for every instance, not only the instance the
 * user happens to launch. The backend performs the disk work as one bounded
 * background task; this hook only owns timing and React state.
 */
export function useInstanceHealthMonitor(enabled = true) {
  const [reports, setReports] = useState<InstanceHealthReports>({});
  const [errors, setErrors] = useState<InstanceHealthErrors>({});
  const refreshInFlight = useRef<Promise<void> | null>(null);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    if (refreshInFlight.current) return refreshInFlight.current;
    const task = (async () => {
      try {
        const results = await checkAllInstanceHealth();
        // Older backends may not expose the batch command during a rolling
        // update. Treat that as an unavailable refresh, not a blank status.
        if (!Array.isArray(results) || !mounted.current) return;
        setReports((current) => {
          const next: InstanceHealthReports = {};
          for (const result of results) {
            // A transient failure keeps the last known status for this still
            // existing instance; a deleted instance disappears because it is
            // absent from the batch result entirely.
            if (result.report) next[result.instance_id] = result.report;
            else if (current[result.instance_id]) next[result.instance_id] = current[result.instance_id];
          }
          return next;
        });
        setErrors((current) => {
          const next: InstanceHealthErrors = {};
          for (const result of results) {
            if (result.error) next[result.instance_id] = result.error;
            else if (!result.report && current[result.instance_id]) next[result.instance_id] = current[result.instance_id];
          }
          return next;
        });
      } catch {
        // Preserve the last known report. Launch-time health remains the final
        // authority if a periodic background scan is temporarily unavailable.
      }
    })();
    refreshInFlight.current = task;
    try {
      await task;
    } finally {
      if (refreshInFlight.current === task) refreshInFlight.current = null;
    }
  }, []);

  const updateReport = useCallback((instanceId: string, report: HealthReport) => {
    setReports((current) => ({ ...current, [instanceId]: report }));
    setErrors((current) => {
      const next = { ...current };
      delete next[instanceId];
      return next;
    });
  }, []);

  useEffect(() => {
    if (!enabled) return undefined;
    mounted.current = true;
    void refresh();
    const interval = window.setInterval(() => void refresh(), HEALTH_SCAN_INTERVAL_MS);
    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') void refresh();
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      mounted.current = false;
      window.clearInterval(interval);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, [enabled, refresh]);

  return { reports, errors, refresh, updateReport };
}
