import { PlusIcon } from "@radix-ui/react-icons";
import type { RefObject } from "react";

import type { SimulatorMetadata } from "../../api/types";
import { simulatorRuntimeLabel } from "./simulatorDisplay";
import { SimulatorRow } from "./SimulatorRow";

interface SimulatorPickerMenuProps {
  filteredSimulators: SimulatorMetadata[];
  hideSimulatorSelection?: boolean;
  isLoading: boolean;
  menuOpen: boolean;
  menuRef: RefObject<HTMLDivElement | null>;
  onChangeSearch: (value: string) => void;
  onCloseMenu: () => void;
  onOpenNewSimulator: () => void;
  onToggleMenu: () => void;
  search: string;
  selectedSimulator: SimulatorMetadata | null;
  selectedSimulatorIdentifier: string;
  setSelectedUDID: (udid: string) => void;
}

export function SimulatorPickerMenu({
  filteredSimulators,
  hideSimulatorSelection = false,
  isLoading,
  menuOpen,
  menuRef,
  onChangeSearch,
  onCloseMenu,
  onOpenNewSimulator,
  onToggleMenu,
  search,
  selectedSimulator,
  selectedSimulatorIdentifier,
  setSelectedUDID,
}: SimulatorPickerMenuProps) {
  const content = selectedSimulator ? (
    <div className="toolbar-sim-copy">
      <div className="toolbar-sim-title-row">
        <span className="toolbar-sim-name">{selectedSimulator.name}</span>
        {selectedSimulator.isBooted ? (
          <span className="state-dot booted toolbar-status-dot" />
        ) : null}
      </div>
      <span className="toolbar-sim-detail">{selectedSimulatorIdentifier}</span>
    </div>
  ) : (
    <span className="toolbar-sim-name muted">
      {isLoading ? "Loading..." : "No simulator selected"}
    </span>
  );

  if (hideSimulatorSelection) {
    return <div className="toolbar-sim-info">{content}</div>;
  }

  return (
    <div className="menu-wrap simulator-picker-wrap" ref={menuRef}>
      <button
        aria-expanded={menuOpen}
        aria-haspopup="menu"
        className={`toolbar-sim-info toolbar-sim-trigger ${
          menuOpen ? "active" : ""
        }`}
        onClick={(event) => {
          event.stopPropagation();
          onToggleMenu();
        }}
        title="Select simulator"
        type="button"
      >
        {content}
      </button>
      {menuOpen ? (
        <div
          className="menu-popover simulator-picker-popover"
          onPointerDown={(event) => event.stopPropagation()}
        >
          <input
            className="sidebar-search"
            onChange={(event) => onChangeSearch(event.target.value)}
            placeholder="Search simulators..."
            value={search}
          />
          <div className="menu-actions menu-actions-compact">
            <button
              className="menu-action menu-primary-action"
              onClick={() => {
                onOpenNewSimulator();
                onCloseMenu();
              }}
              type="button"
            >
              <PlusIcon />
              New Simulator
            </button>
          </div>
          <div className="sim-list">
            {isLoading ? <p className="list-empty">Loading...</p> : null}
            {!isLoading && filteredSimulators.length === 0 ? (
              <p className="list-empty">No matches</p>
            ) : null}
            {filteredSimulators.map((simulator) => (
              <SimulatorRow
                isSelected={simulator.udid === selectedSimulator?.udid}
                key={simulator.udid}
                onSelect={() => {
                  setSelectedUDID(simulator.udid);
                  onCloseMenu();
                }}
                simulator={simulator}
              />
            ))}
          </div>
          {selectedSimulator ? (
            <div className="simulator-picker-current">
              {simulatorRuntimeLabel(selectedSimulator)}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
