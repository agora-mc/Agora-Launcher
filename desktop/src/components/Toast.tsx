import { useEffect, useState } from 'react';

interface ToastData {
  id: number;
  message: string;
  type: 'success' | 'error';
}

let nextId = 0;
let addToastFn: ((t: ToastData) => void) | null = null;

export function showToast(message: string, type: 'success' | 'error' = 'success') {
  addToastFn?.({ id: ++nextId, message, type });
}

export function ToastContainer() {
  const [toasts, setToasts] = useState<ToastData[]>([]);

  useEffect(() => {
    addToastFn = (t) => setToasts((prev) => [...prev, t]);
    return () => { addToastFn = null; };
  }, []);

  const remove = (id: number) => setToasts((prev) => prev.filter((t) => t.id !== id));

  return (
    <div className="fixed bottom-4 right-4 z-100 flex flex-col gap-2 max-w-sm">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} onDone={remove} />
      ))}
    </div>
  );
}

function ToastItem({ toast, onDone }: { toast: ToastData; onDone: (id: number) => void }) {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    const timer = setTimeout(() => {
      setVisible(false);
      setTimeout(() => onDone(toast.id), 300);
    }, 3000);
    return () => clearTimeout(timer);
  }, [toast.id, onDone]);

  return (
    <div
      className={`rounded-lg px-4 py-2 text-sm shadow-lg transition-opacity duration-300 ${
        visible ? 'opacity-100' : 'opacity-0'
      } ${
        toast.type === 'error'
          ? 'bg-destructive text-destructive-foreground'
          : 'bg-primary text-primary-foreground'
      }`}
      role="alert"
    >
      {toast.message}
    </div>
  );
}
