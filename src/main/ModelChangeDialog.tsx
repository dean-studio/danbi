import { useEffect, useState } from "react";
import { Dialog } from "@/components/Dialog";
import {
  CurrentSelectionCard,
  ModelPicker,
  filterInvokable,
  sortBedrockModels,
} from "@/components/ModelPicker";
import {
  PrimaryButton,
  SecondaryButton,
} from "@/components/WizardShell";
import {
  ipc,
  type AuthInput,
  type BedrockModel,
  type ModelInfo,
} from "@/lib/ipc";
import { useApp } from "@/state/store";

export function ModelChangeDialog({
  open,
  role,
  onClose,
}: {
  open: boolean;
  role: "routing" | "writer" | null;
  onClose: () => void;
}) {
  const cfg = useApp((s) => s.cfg);
  const setCfg = useApp((s) => s.setCfg);

  const [loading, setLoading] = useState(false);
  const [models, setModels] = useState<BedrockModel[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [pick, setPick] = useState<string>("");

  useEffect(() => {
    if (!open || !role || !cfg?.provider) return;
    const provider = cfg.provider;

    setPick(
      (role === "routing" ? cfg.models.routing : cfg.models.writer) ?? "",
    );

    // Non-Bedrock providers just expose their curated catalog.
    const loadStatic = (loader: () => Promise<ModelInfo[]>) => {
      setLoading(true);
      setErr(null);
      setModels(null);
      loader()
        .then((list) =>
          setModels(
            list.map((m) => ({
              id: m.id,
              name: m.name,
              provider: m.provider,
              on_demand: m.on_demand,
              modalities_in: m.modalities_in,
              modalities_out: m.modalities_out,
            })),
          ),
        )
        .catch((e) => setErr(String(e)))
        .finally(() => setLoading(false));
    };

    switch (provider.kind) {
      case "nvidia":
        loadStatic(() => ipc.listNvidiaModels());
        return;
      case "openai":
        loadStatic(() => ipc.listOpenaiModels());
        return;
      case "anthropic":
        loadStatic(() => ipc.listAnthropicModels());
        return;
      case "google":
        loadStatic(() => ipc.listGoogleModels());
        return;
      case "ollama":
        loadStatic(() =>
          ipc.listOllamaModels(provider.base_url ?? undefined),
        );
        return;
      case "voyage":
        // Voyage is embedding-only; the model-change dialog targets
        // chat/Writer models so we just bail here rather than listing
        // unusable options.
        setModels([]);
        setLoading(false);
        return;
      case "bedrock":
        break;
      default:
        break;
    }

    if (provider.kind !== "bedrock") return;

    const auth: AuthInput =
      provider.auth_mode === "profile"
        ? { kind: "profile", name: provider.profile ?? "default" }
        : provider.auth_mode === "manual"
          ? { kind: "manual", label: "manual-default" }
          : { kind: "env" };

    setLoading(true);
    setErr(null);
    setModels(null);

    ipc
      .listBedrockModels(auth, provider.region)
      .then((list) => {
        const sentinel = list.find((m) => m.id.startsWith("__error__"));
        if (sentinel?.name) setErr(sentinel.name);
        setModels(list.filter((m) => !m.id.startsWith("__error__")));
      })
      .catch((e) => setErr(String(e)))
      .finally(() => setLoading(false));
  }, [open, role, cfg]);

  if (!role) return null;

  const sorted = sortBedrockModels(models ?? []);
  const options = filterInvokable(sorted);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={role === "routing" ? "Routing 모델 변경" : "Writer 모델 변경"}
      width={520}
      footer={
        <>
          <SecondaryButton onClick={onClose}>취소</SecondaryButton>
          <PrimaryButton
            disabled={!pick || !cfg?.vault_path}
            onClick={async () => {
              if (!cfg?.vault_path) return;
              const nextModels =
                role === "routing"
                  ? { ...cfg.models, routing: pick }
                  : { ...cfg.models, writer: pick };
              const nextCfg = { ...cfg, models: nextModels };
              await ipc.saveConfig(cfg.vault_path, nextCfg);
              setCfg(nextCfg);
              onClose();
            }}
          >
            저장
          </PrimaryButton>
        </>
      }
    >
      <div className="mb-3">
        <CurrentSelectionCard
          roleLabel={role === "routing" ? "ROUTE" : "WRITE"}
          id={pick || null}
          meta={
            pick
              ? cfg?.models[role] === pick
                ? "현재 설정"
                : "변경됨 (저장 전)"
              : undefined
          }
        />
      </div>
      {loading && (
        <div className="text-[13px] text-mute">모델 목록을 불러오는 중…</div>
      )}
      {err && (
        <div className="mb-3 rounded-md border border-hairline bg-surface-elevated p-3 text-[12px] text-mute">
          {err}
        </div>
      )}
      {!loading && <ModelPicker value={pick} onChange={setPick} options={options} />}
    </Dialog>
  );
}
