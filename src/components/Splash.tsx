import { motion } from "motion/react";

/** Startup screen: the app's two quota rings, spinning while the accounts load. */
export function Splash() {
  return (
    <motion.div
      className="splash"
      data-tauri-drag-region
      role="status"
      aria-label="Chargement"
      initial={{ opacity: 1 }}
      exit={{ opacity: 0, scale: 1.02 }}
      transition={{ duration: 0.35, ease: "easeOut" }}
    >
      <motion.div
        className="splash-mark"
        initial={{ opacity: 0, scale: 0.92 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.45, ease: [0.2, 0.8, 0.2, 1] }}
      >
        <svg viewBox="0 0 120 120" width="112" height="112" aria-hidden>
          <circle className="splash-track" cx="60" cy="60" r="50" />
          <circle className="splash-track" cx="60" cy="60" r="34" />
          <circle className="splash-arc outer" cx="60" cy="60" r="50" pathLength="100" />
          <circle className="splash-arc inner" cx="60" cy="60" r="34" pathLength="100" />
        </svg>
      </motion.div>
      <motion.div
        className="splash-text"
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.45, delay: 0.12 }}
      >
        <span className="splash-brand">Trigenys</span>
        <span className="splash-title">Claude Accounts Manager</span>
        <span className="splash-status">Chargement des comptes…</span>
      </motion.div>
    </motion.div>
  );
}
