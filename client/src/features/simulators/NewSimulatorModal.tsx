import { useEffect, useMemo, useState, type FormEvent } from "react";

import {
  createSimulator,
  fetchSimulatorCreateOptions,
} from "../../api/simulators";
import type {
  AndroidEmulatorDeviceTypeOption,
  AndroidEmulatorSystemImageOption,
  CreateSimulatorResponse,
  SimulatorCreateOptionsResponse,
  SimulatorDeviceTypeOption,
  SimulatorMetadata,
  SimulatorRuntimeOption,
} from "../../api/types";

interface NewSimulatorModalProps {
  onClose: () => void;
  onCreated: (response: CreateSimulatorResponse) => void;
  open: boolean;
  selectedSimulator: SimulatorMetadata | null;
}

type ModalStep = "simulator" | "watch";
type CreationPlatform = "ios" | "android";

export function NewSimulatorModal({
  onClose,
  onCreated,
  open,
  selectedSimulator,
}: NewSimulatorModalProps) {
  const [options, setOptions] = useState<SimulatorCreateOptionsResponse | null>(
    null,
  );
  const [platform, setPlatform] = useState<CreationPlatform>(
    selectedSimulator?.platform === "android-emulator" ? "android" : "ios",
  );
  const [deviceTypeIdentifier, setDeviceTypeIdentifier] = useState("");
  const [runtimeIdentifier, setRuntimeIdentifier] = useState("");
  const [name, setName] = useState("");
  const [nameDirty, setNameDirty] = useState(false);
  const [androidDeviceTypeIdentifier, setAndroidDeviceTypeIdentifier] =
    useState("");
  const [androidSystemImageIdentifier, setAndroidSystemImageIdentifier] =
    useState("");
  const [androidName, setAndroidName] = useState("");
  const [androidNameDirty, setAndroidNameDirty] = useState(false);
  const [pairedWatch, setPairedWatch] = useState(false);
  const [watchDeviceTypeIdentifier, setWatchDeviceTypeIdentifier] =
    useState("");
  const [watchRuntimeIdentifier, setWatchRuntimeIdentifier] = useState("");
  const [watchName, setWatchName] = useState("");
  const [watchNameDirty, setWatchNameDirty] = useState(false);
  const [step, setStep] = useState<ModalStep>("simulator");
  const [error, setError] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const selectedDeviceTypeIdentifier =
    selectedSimulator?.platform === "android-emulator"
      ? ""
      : (selectedSimulator?.deviceTypeIdentifier ?? "");
  const selectedRuntimeIdentifier = selectedSimulator?.runtimeIdentifier ?? "";

  useEffect(() => {
    if (!open) {
      return;
    }

    let cancelled = false;
    const controller =
      typeof AbortController !== "undefined" ? new AbortController() : null;

    setError("");
    setIsLoading(true);
    setIsCreating(false);
    setOptions(null);
    setStep("simulator");

    void fetchSimulatorCreateOptions(
      controller ? { signal: controller.signal } : {},
    )
      .then((nextOptions) => {
        if (cancelled) {
          return;
        }
        setOptions(nextOptions);
        const initialPlatform =
          selectedSimulator?.platform === "android-emulator" ? "android" : "ios";
        setPlatform(initialPlatform);
        const initialDeviceType = chooseInitialDeviceType(
          nextOptions.deviceTypes,
          selectedDeviceTypeIdentifier,
        );
        const initialRuntime = chooseCompatibleRuntime(
          initialDeviceType?.identifier ?? "",
          nextOptions,
          selectedRuntimeIdentifier,
        );
        setDeviceTypeIdentifier(initialDeviceType?.identifier ?? "");
        setRuntimeIdentifier(initialRuntime?.identifier ?? "");
        setName(initialDeviceType?.name ?? "");
        setNameDirty(false);

        const initialWatchDeviceType = chooseInitialWatchDeviceType(
          nextOptions,
        );
        const initialWatchRuntime = chooseCompatibleRuntime(
          initialWatchDeviceType?.identifier ?? "",
          nextOptions,
        );
        setWatchDeviceTypeIdentifier(initialWatchDeviceType?.identifier ?? "");
        setWatchRuntimeIdentifier(initialWatchRuntime?.identifier ?? "");
        setWatchName(initialWatchDeviceType?.name ?? "");
        setWatchNameDirty(false);
        setPairedWatch(false);

        const initialAndroidDeviceType = chooseInitialAndroidDeviceType(
          nextOptions,
          selectedSimulator?.android?.avdName,
        );
        const initialAndroidSystemImage =
          chooseInitialAndroidSystemImage(nextOptions);
        setAndroidDeviceTypeIdentifier(
          initialAndroidDeviceType?.identifier ?? "",
        );
        setAndroidSystemImageIdentifier(
          initialAndroidSystemImage?.identifier ?? "",
        );
        setAndroidName(
          initialAndroidDeviceType
            ? defaultAndroidName(
                initialAndroidDeviceType,
                initialAndroidSystemImage,
              )
            : "",
        );
        setAndroidNameDirty(false);
      })
      .catch((loadError) => {
        if (
          !cancelled &&
          !(loadError instanceof DOMException && loadError.name === "AbortError")
        ) {
          setError(
            loadError instanceof Error
              ? loadError.message
              : "Failed to load simulator options.",
          );
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false);
        }
      });

    return () => {
      cancelled = true;
      controller?.abort();
    };
  }, [
    open,
    selectedDeviceTypeIdentifier,
    selectedRuntimeIdentifier,
    selectedSimulator?.android?.avdName,
    selectedSimulator?.platform,
  ]);

  useEffect(() => {
    if (!open) {
      return;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  const selectedDeviceType = options?.deviceTypes.find(
    (deviceType) => deviceType.identifier === deviceTypeIdentifier,
  );
  const androidDeviceTypes = options?.android?.deviceTypes ?? [];
  const androidSystemImages = options?.android?.systemImages ?? [];
  const selectedAndroidDeviceType = androidDeviceTypes.find(
    (deviceType) => deviceType.identifier === androidDeviceTypeIdentifier,
  );
  const selectedAndroidSystemImage = androidSystemImages.find(
    (systemImage) => systemImage.identifier === androidSystemImageIdentifier,
  );
  const runtimeOptions = useMemo(
    () => compatibleRuntimes(deviceTypeIdentifier, options),
    [deviceTypeIdentifier, options],
  );
  const watchDeviceTypes = useMemo(
    () =>
      (options?.deviceTypes ?? []).filter(
        (deviceType) =>
          isWatchDeviceType(deviceType) &&
          compatibleRuntimes(deviceType.identifier, options).length > 0,
      ),
    [options],
  );
  const watchRuntimeOptions = useMemo(
    () => compatibleRuntimes(watchDeviceTypeIdentifier, options),
    [options, watchDeviceTypeIdentifier],
  );
  const pairedWatchAvailable =
    selectedDeviceType != null &&
    isPhoneDeviceType(selectedDeviceType) &&
    watchDeviceTypes.length > 0 &&
    watchRuntimeOptions.length > 0;
  const deviceTypeGroups = useMemo(
    () => groupDeviceTypes(options?.deviceTypes ?? []),
    [options],
  );
  const watchDeviceTypeGroups = useMemo(
    () => groupDeviceTypes(watchDeviceTypes),
    [watchDeviceTypes],
  );
  const trimmedName = name.trim();
  const trimmedAndroidName = androidName.trim();
  const trimmedWatchName = watchName.trim();
  const canCreateAndroid =
    Boolean(
      trimmedAndroidName &&
        androidDeviceTypeIdentifier &&
        androidSystemImageIdentifier,
    ) && !options?.android?.unavailableReason;
  const canCreateSimulator =
    platform === "android"
      ? canCreateAndroid
      : Boolean(trimmedName && deviceTypeIdentifier && runtimeIdentifier) &&
        (!pairedWatch ||
          Boolean(
            trimmedWatchName &&
              watchDeviceTypeIdentifier &&
              watchRuntimeIdentifier,
          ));

  useEffect(() => {
    if (!options || !deviceTypeIdentifier) {
      return;
    }
    const nextRuntime = chooseCompatibleRuntime(
      deviceTypeIdentifier,
      options,
      runtimeIdentifier,
    );
    if (nextRuntime?.identifier !== runtimeIdentifier) {
      setRuntimeIdentifier(nextRuntime?.identifier ?? "");
    }
  }, [deviceTypeIdentifier, options, runtimeIdentifier]);

  useEffect(() => {
    if (!options || !watchDeviceTypeIdentifier) {
      return;
    }
    const nextRuntime = chooseCompatibleRuntime(
      watchDeviceTypeIdentifier,
      options,
      watchRuntimeIdentifier,
    );
    if (nextRuntime?.identifier !== watchRuntimeIdentifier) {
      setWatchRuntimeIdentifier(nextRuntime?.identifier ?? "");
    }
  }, [options, watchDeviceTypeIdentifier, watchRuntimeIdentifier]);

  useEffect(() => {
    if (!pairedWatchAvailable) {
      setPairedWatch(false);
      setStep("simulator");
    }
  }, [pairedWatchAvailable]);

  if (!open) {
    return null;
  }

  function handlePlatformChange(nextPlatform: CreationPlatform) {
    setPlatform(nextPlatform);
    setStep("simulator");
    setError("");
    if (nextPlatform === "android") {
      setPairedWatch(false);
    }
  }

  function handleDeviceTypeChange(nextIdentifier: string) {
    setDeviceTypeIdentifier(nextIdentifier);
    const nextDeviceType = options?.deviceTypes.find(
      (deviceType) => deviceType.identifier === nextIdentifier,
    );
    const nextRuntime = chooseCompatibleRuntime(nextIdentifier, options);
    setRuntimeIdentifier(nextRuntime?.identifier ?? "");
    if (!nameDirty) {
      setName(nextDeviceType?.name ?? "");
    }
  }

  function handleWatchDeviceTypeChange(nextIdentifier: string) {
    setWatchDeviceTypeIdentifier(nextIdentifier);
    const nextDeviceType = options?.deviceTypes.find(
      (deviceType) => deviceType.identifier === nextIdentifier,
    );
    const nextRuntime = chooseCompatibleRuntime(nextIdentifier, options);
    setWatchRuntimeIdentifier(nextRuntime?.identifier ?? "");
    if (!watchNameDirty) {
      setWatchName(nextDeviceType?.name ?? "");
    }
  }

  function handleAndroidDeviceTypeChange(nextIdentifier: string) {
    setAndroidDeviceTypeIdentifier(nextIdentifier);
    const nextDeviceType = androidDeviceTypes.find(
      (deviceType) => deviceType.identifier === nextIdentifier,
    );
    if (!androidNameDirty && nextDeviceType) {
      setAndroidName(
        defaultAndroidName(nextDeviceType, selectedAndroidSystemImage),
      );
    }
  }

  function handleAndroidSystemImageChange(nextIdentifier: string) {
    setAndroidSystemImageIdentifier(nextIdentifier);
    const nextSystemImage = androidSystemImages.find(
      (systemImage) => systemImage.identifier === nextIdentifier,
    );
    if (!androidNameDirty && selectedAndroidDeviceType) {
      setAndroidName(defaultAndroidName(selectedAndroidDeviceType, nextSystemImage));
    }
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError("");
    if (platform === "ios" && step === "simulator" && pairedWatch) {
      setStep("watch");
      return;
    }
    void create();
  }

  async function create() {
    if (!canCreateSimulator) {
      setError(
        platform === "android"
          ? "Choose a name, device profile, and system image."
          : "Choose a name, device type, and OS version.",
      );
      return;
    }
    setIsCreating(true);
    try {
      const response = await createSimulator({
        deviceTypeIdentifier:
          platform === "android"
            ? androidDeviceTypeIdentifier
            : deviceTypeIdentifier,
        name: platform === "android" ? trimmedAndroidName : trimmedName,
        pairedWatch: platform === "ios" && pairedWatch
          ? {
              deviceTypeIdentifier: watchDeviceTypeIdentifier,
              name: trimmedWatchName,
              runtimeIdentifier: watchRuntimeIdentifier,
            }
          : undefined,
        platform,
        runtimeIdentifier:
          platform === "android" ? androidSystemImageIdentifier : runtimeIdentifier,
      });
      onCreated(response);
    } catch (createError) {
      setError(
        createError instanceof Error
          ? createError.message
          : "Failed to create simulator.",
      );
    } finally {
      setIsCreating(false);
    }
  }

  return (
    <div
      className="new-sim-overlay"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <form
        aria-labelledby="new-sim-title"
        aria-modal="true"
        className="new-sim-window"
        onSubmit={handleSubmit}
        role="dialog"
      >
        <div className="new-sim-titlebar">
          <span className="new-sim-window-controls" aria-hidden="true">
            <span className="new-sim-window-dot close" />
            <span className="new-sim-window-dot minimize" />
            <span className="new-sim-window-dot zoom" />
          </span>
          <h2 id="new-sim-title">New Simulator</h2>
        </div>

        <div className="new-sim-body">
          <div className="new-sim-platform-switcher" aria-label="Platform">
            <button
              className={platform === "ios" ? "active" : ""}
              onClick={() => handlePlatformChange("ios")}
              type="button"
            >
              iOS
            </button>
            <button
              className={platform === "android" ? "active" : ""}
              onClick={() => handlePlatformChange("android")}
              type="button"
            >
              Android
            </button>
          </div>
          <fieldset className="new-sim-fieldset" disabled={isCreating}>
            {isLoading ? (
              <p className="new-sim-status">Loading simulator options...</p>
            ) : platform === "android" ? (
              <>
                {options?.android?.unavailableReason ? (
                  <p className="new-sim-status">
                    {options.android.unavailableReason}
                  </p>
                ) : null}
                <label className="new-sim-field">
                  <span>Emulator Name:</span>
                  <input
                    autoFocus
                    onChange={(event) => {
                      setAndroidName(event.currentTarget.value);
                      setAndroidNameDirty(true);
                    }}
                    value={androidName}
                  />
                </label>
                <label className="new-sim-field">
                  <span>Device Profile:</span>
                  <select
                    onChange={(event) =>
                      handleAndroidDeviceTypeChange(event.currentTarget.value)
                    }
                    value={androidDeviceTypeIdentifier}
                  >
                    {androidDeviceTypes.map((deviceType) => (
                      <option
                        key={deviceType.identifier}
                        value={deviceType.identifier}
                      >
                        {deviceType.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="new-sim-field">
                  <span>System Image:</span>
                  <select
                    onChange={(event) =>
                      handleAndroidSystemImageChange(event.currentTarget.value)
                    }
                    value={androidSystemImageIdentifier}
                  >
                    {androidSystemImages.map((systemImage) => (
                      <option
                        key={systemImage.identifier}
                        value={systemImage.identifier}
                      >
                        {systemImage.name}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            ) : step === "watch" ? (
              <>
                <label className="new-sim-field">
                  <span>Paired Watch Name:</span>
                  <input
                    autoFocus
                    onChange={(event) => {
                      setWatchName(event.currentTarget.value);
                      setWatchNameDirty(true);
                    }}
                    value={watchName}
                  />
                </label>
                <label className="new-sim-field">
                  <span>Device Type:</span>
                  <select
                    onChange={(event) =>
                      handleWatchDeviceTypeChange(event.currentTarget.value)
                    }
                    value={watchDeviceTypeIdentifier}
                  >
                    {renderDeviceTypeGroups(watchDeviceTypeGroups)}
                  </select>
                </label>
                <label className="new-sim-field">
                  <span>OS Version:</span>
                  <select
                    onChange={(event) =>
                      setWatchRuntimeIdentifier(event.currentTarget.value)
                    }
                    value={watchRuntimeIdentifier}
                  >
                    {watchRuntimeOptions.map((runtime) => (
                      <option
                        key={runtime.identifier}
                        value={runtime.identifier}
                      >
                        {runtime.name}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            ) : (
              <>
                <label className="new-sim-field">
                  <span>Simulator Name:</span>
                  <input
                    autoFocus
                    onChange={(event) => {
                      setName(event.currentTarget.value);
                      setNameDirty(true);
                    }}
                    value={name}
                  />
                </label>
                <label className="new-sim-field">
                  <span>Device Type:</span>
                  <select
                    onChange={(event) =>
                      handleDeviceTypeChange(event.currentTarget.value)
                    }
                    value={deviceTypeIdentifier}
                  >
                    {renderDeviceTypeGroups(deviceTypeGroups)}
                  </select>
                </label>
                <label className="new-sim-field">
                  <span>OS Version:</span>
                  <select
                    onChange={(event) =>
                      setRuntimeIdentifier(event.currentTarget.value)
                    }
                    value={runtimeIdentifier}
                  >
                    {runtimeOptions.map((runtime) => (
                      <option
                        key={runtime.identifier}
                        value={runtime.identifier}
                      >
                        {runtime.name}
                      </option>
                    ))}
                  </select>
                </label>
                {pairedWatchAvailable ? (
                  <label className="new-sim-checkbox">
                    <input
                      checked={pairedWatch}
                      onChange={(event) =>
                        setPairedWatch(event.currentTarget.checked)
                      }
                      type="checkbox"
                    />
                    <span>Paired Apple Watch</span>
                  </label>
                ) : null}
              </>
            )}
          </fieldset>
          {error ? <p className="new-sim-error">{error}</p> : null}
        </div>

        <div className="new-sim-actions">
          <button
            className="new-sim-button"
            disabled={isCreating}
            onClick={onClose}
            type="button"
          >
            Cancel
          </button>
          <span className="new-sim-action-spacer" />
          <button
            className="new-sim-button"
            disabled={platform === "android" || step === "simulator" || isCreating}
            onClick={() => setStep("simulator")}
            type="button"
          >
            Previous
          </button>
          <button
            className="new-sim-button"
            disabled={
              isLoading ||
              isCreating ||
              !canCreateSimulator ||
              (platform === "ios" &&
                step === "simulator" &&
                pairedWatch &&
                !pairedWatchAvailable)
            }
            type="submit"
          >
            {platform === "ios" && step === "simulator" && pairedWatch
              ? "Next"
              : isCreating
                ? "Creating..."
                : "Create"}
          </button>
        </div>
      </form>
    </div>
  );
}

function chooseInitialAndroidDeviceType(
  options: SimulatorCreateOptionsResponse,
  preferredName?: string,
) {
  const deviceTypes = options.android?.deviceTypes ?? [];
  return (
    deviceTypes.find((deviceType) => deviceType.identifier === preferredName) ??
    deviceTypes.find((deviceType) => deviceType.identifier === "pixel_8") ??
    deviceTypes.find((deviceType) => deviceType.identifier.startsWith("pixel_")) ??
    deviceTypes[0]
  );
}

function chooseInitialAndroidSystemImage(options: SimulatorCreateOptionsResponse) {
  return options.android?.systemImages?.[0];
}

function defaultAndroidName(
  deviceType: AndroidEmulatorDeviceTypeOption,
  systemImage?: AndroidEmulatorSystemImageOption,
) {
  const apiSuffix = systemImage?.apiLevel ? `_API_${systemImage.apiLevel}` : "";
  return `${deviceType.name}${apiSuffix}`
    .replace(/[^A-Za-z0-9_.-]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function chooseInitialDeviceType(
  deviceTypes: SimulatorDeviceTypeOption[],
  selectedDeviceTypeIdentifier: string,
) {
  return (
    deviceTypes.find(
      (deviceType) => deviceType.identifier === selectedDeviceTypeIdentifier,
    ) ??
    deviceTypes.find((deviceType) => isPhoneDeviceType(deviceType)) ??
    deviceTypes.find((deviceType) => !isWatchDeviceType(deviceType)) ??
    deviceTypes[0]
  );
}

function chooseInitialWatchDeviceType(options: SimulatorCreateOptionsResponse) {
  return options.deviceTypes.find(
    (deviceType) =>
      isWatchDeviceType(deviceType) &&
      compatibleRuntimes(deviceType.identifier, options).length > 0,
  );
}

function chooseCompatibleRuntime(
  deviceTypeIdentifier: string,
  options: SimulatorCreateOptionsResponse | null,
  preferredIdentifier?: string,
) {
  const runtimes = compatibleRuntimes(deviceTypeIdentifier, options);
  return (
    runtimes.find((runtime) => runtime.identifier === preferredIdentifier) ??
    runtimes[0]
  );
}

function compatibleRuntimes(
  deviceTypeIdentifier: string,
  options: SimulatorCreateOptionsResponse | null,
): SimulatorRuntimeOption[] {
  if (!deviceTypeIdentifier || !options) {
    return [];
  }
  const deviceType = options.deviceTypes.find(
    (candidate) => candidate.identifier === deviceTypeIdentifier,
  );
  return options.runtimes.filter((runtime) => {
    if (runtime.isAvailable === false) {
      return false;
    }
    return (
      runtime.supportedDeviceTypeIdentifiers?.includes(deviceTypeIdentifier) ||
      deviceType?.supportedRuntimeIdentifiers?.includes(runtime.identifier)
    );
  });
}

function groupDeviceTypes(deviceTypes: SimulatorDeviceTypeOption[]) {
  const groups: Array<{
    family: string;
    deviceTypes: SimulatorDeviceTypeOption[];
  }> = [];
  for (const deviceType of deviceTypes) {
    const family = deviceType.productFamily ?? "Other";
    let group = groups.find((candidate) => candidate.family === family);
    if (!group) {
      group = { deviceTypes: [], family };
      groups.push(group);
    }
    group.deviceTypes.push(deviceType);
  }
  return groups;
}

function renderDeviceTypeGroups(
  groups: Array<{
    family: string;
    deviceTypes: SimulatorDeviceTypeOption[];
  }>,
) {
  return groups.map((group) => (
    <optgroup key={group.family} label={group.family}>
      {group.deviceTypes.map((deviceType) => (
        <option key={deviceType.identifier} value={deviceType.identifier}>
          {deviceType.name}
        </option>
      ))}
    </optgroup>
  ));
}

function isPhoneDeviceType(deviceType: SimulatorDeviceTypeOption): boolean {
  return (deviceType.productFamily ?? "").toLowerCase() === "iphone";
}

function isWatchDeviceType(deviceType: SimulatorDeviceTypeOption): boolean {
  return (deviceType.productFamily ?? "").toLowerCase().includes("watch");
}
