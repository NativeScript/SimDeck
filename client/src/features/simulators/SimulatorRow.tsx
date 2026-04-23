import type { SimulatorMetadata } from "../../api/types";
import { simulatorRuntimeLabel } from "./simulatorDisplay";

interface SimulatorRowProps {
  isSelected: boolean;
  onSelect: () => void;
  simulator: SimulatorMetadata;
}

export function SimulatorRow({
  isSelected,
  onSelect,
  simulator,
}: SimulatorRowProps) {
  return (
    <button
      className={`sim-item ${isSelected ? "selected" : ""}`}
      onClick={onSelect}
    >
      <span className="sim-item-name">{simulator.name}</span>
      <span className="sim-item-meta">
        {simulatorRuntimeLabel(simulator)}
        <span className={`state-dot ${simulator.isBooted ? "booted" : ""}`} />
      </span>
    </button>
  );
}
