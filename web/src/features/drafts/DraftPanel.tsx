import { type FormEvent, useEffect, useRef, useState } from "react";
import type { MapNode, MapRef, NodeKind, Overview, Seam } from "../../protocol.ts";
import { ApiError, failureText } from "../../shared/api.ts";
import { relative } from "../../shared/format.ts";
import { type MapOp, draftsApi } from "./api.ts";
import { type DiffLine, changedWithContext, lineDiff } from "./diff.ts";
import { flowsFromText, flowsToText, opSummary } from "./overlay.ts";

interface Props {
  overview: Overview;
  /** The part or seam selected on the map: its form is shown. */
  selection: MapRef | null;
  /** Something changed on the server: fetch the overview again. */
  onChanged: () => void | Promise<void>;
  /** Say something in the shell's notice, with one way back. */
  onNotice: (text: string, undo?: () => void) => void;
  /** Set by a double-click on the canvas: the Add part form takes focus. */
  focusAddPart?: number | undefined;
  /** A part was added: the shell may place it where the canvas was double-clicked. */
  onPartAdded?: ((id: string) => void) | undefined;
}

const KINDS: NodeKind[] = ["context", "contract", "external"];
/** An empty field is "not given"; `compact` then leaves the key out. */
const blank = (s: string): string | null => (s.trim() === "" ? null : s.trim());

/** `{a: undefined}` must not be sent: the server would read it as given. */
function compact<T extends object>(o: T): T {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== null && !(Array.isArray(v) && v.length === 0))) as T;
}

/**
 * The shared draft of the map (design S18 to S25): what the team has changed, the forms that
 * change it, and the way to propose it as a pull request, withdraw it, discard it or copy it.
 * Every write goes to the server; what is shown comes back with the overview, so every browser
 * shows the same draft.
 */
export function DraftPanel({ overview, selection, onChanged, onNotice, focusAddPart, onPartAdded }: Props) {
  const draft = overview.draft ?? {};
  const ops = draft.ops ?? [];
  const proposal = draft.proposal ?? null;
  const frozen = proposal !== null;
  const map = draft.preview ?? overview.map.map;
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [review, setReview] = useState<{ diff: DiffLine[]; yaml: string } | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const addPartId = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (focusAddPart) {
      addPartId.current?.focus();
    }
  }, [focusAddPart]);

  /** Run one write; a refusal shows the server's own reason next to the forms. */
  const run = async (what: string, action: () => Promise<unknown>): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await onChanged();
      return true;
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 401)) {
        setError(`${what}: ${failureText(e)}`);
      }
      // After a refusal the draft shown is the draft as it is (someone else may have changed it).
      await onChanged();
      return false;
    } finally {
      setBusy(false);
    }
  };
  const post = (op: MapOp, what: string) => run(what, () => draftsApi.addOp(op));

  const addPart = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = e.currentTarget;
    const f = new FormData(form);
    const id = String(f.get("id") ?? "").trim();
    const node: MapNode = compact({ id, kind: String(f.get("kind")) as NodeKind, description: blank(String(f.get("description") ?? "")), repo: blank(String(f.get("repo") ?? "")), path: blank(String(f.get("path") ?? "")) });
    if (await post({ op: "add_node", node }, `Part ${id} was not added`)) {
      form.reset();
      onPartAdded?.(id);
    }
  };

  const addSeam = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = e.currentTarget;
    const f = new FormData(form);
    const from = String(f.get("from"));
    const to = String(f.get("to"));
    const id = blank(String(f.get("id") ?? "")) ?? `${from}-${to}`;
    const seam: Seam = compact({ id, from, to, owner: String(f.get("owner")) === "from" ? from : to, carries: blank(String(f.get("carries") ?? "")) });
    if (await post({ op: "add_seam", seam }, `Seam ${id} was not added`)) {
      form.reset();
    }
  };

  const selectedNode = selection?.kind === "node" ? (map.nodes.find((n) => n.id === selection.id) ?? null) : null;
  const selectedSeam = selection?.kind === "seam" ? ((map.seams ?? []).find((s) => s.id === selection.id) ?? null) : null;

  const updatePart = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!selectedNode) {
      return;
    }
    const f = new FormData(e.currentTarget);
    const node: MapNode = compact({ id: selectedNode.id, kind: String(f.get("kind")) as NodeKind, description: blank(String(f.get("description") ?? "")), repo: blank(String(f.get("repo") ?? "")), path: blank(String(f.get("path") ?? "")) });
    await post({ op: "update_node", node }, `Part ${node.id} was not changed`);
  };

  const updateSeam = async (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!selectedSeam) {
      return;
    }
    const f = new FormData(e.currentTarget);
    let flows: Seam["flows"];
    try {
      flows = flowsFromText(String(f.get("flows") ?? ""));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      return;
    }
    const seam: Seam = compact({
      id: selectedSeam.id,
      from: selectedSeam.from,
      to: selectedSeam.to,
      owner: String(f.get("owner")),
      carries: blank(String(f.get("carries") ?? "")),
      guard: blank(String(f.get("guard") ?? "")),
      description: blank(String(f.get("description") ?? "")),
      flows,
    });
    await post({ op: "update_seam", seam }, `Seam ${seam.id} was not changed`);
  };

  const openReview = () =>
    run("The proposal could not be prepared", async () => {
      const [base, proposed] = await Promise.all([draftsApi.yaml(true), draftsApi.yaml()]);
      setReview({ diff: changedWithContext(lineDiff(base, proposed)), yaml: proposed });
    });

  const propose = async () => {
    const ok = await run("The map change was not proposed", async () => {
      const r = await draftsApi.propose();
      onNotice(`Map change proposed${r.proposal.pr_number ? ` as pull request #${r.proposal.pr_number}` : ""}. The draft is read-only until it is merged, closed or withdrawn.`);
    });
    if (ok) {
      setReview(null);
    }
  };

  const withdraw = () =>
    run("The proposal was not withdrawn", async () => {
      await draftsApi.withdraw();
      onNotice("Proposal withdrawn. The draft is editable again, with every change kept.");
    });

  const restore = () =>
    run("The draft was not brought back", async () => {
      await draftsApi.restore();
      onNotice("The draft is back, with every change and its author.");
    });

  const discard = () =>
    run("The draft was not discarded", async () => {
      await draftsApi.discard();
      onNotice(`Draft discarded for everyone: ${ops.length} change${ops.length === 1 ? "" : "s"}.`, () => void restore());
    });

  const copyYaml = () =>
    run("The proposed file could not be read", async () => {
      const text = await draftsApi.yaml();
      setCopied(text);
      try {
        await navigator.clipboard.writeText(text);
        onNotice("The proposed map file is on the clipboard.");
      } catch {
        // No clipboard permission: the text is shown below to copy by hand.
      }
    });

  const stale = ops.filter((o) => o.stale);
  const proposalText = proposal ? { opening: "A proposal is being opened", proposed: "Proposed", merged: "Merged; the draft empties when the map has been loaded again" }[proposal.state] : null;

  return (
    <section className="draft-panel" data-testid="draft-panel" aria-label="Map draft">
      <header>
        <h2>Map draft</h2>
        <span className="muted">{ops.length === 0 ? "empty" : `${ops.length} change${ops.length === 1 ? "" : "s"}`}</span>
      </header>
      <p className="muted">One draft for the whole team. Nothing here is on the map until it is proposed as a pull request and merged.</p>

      {proposal && (
        <p className={proposal.state === "proposed" ? "notice-line" : "warn"} data-testid="proposal">
          {proposalText}
          {proposal.pr_url && (
            <>
              {" · "}
              <a href={proposal.pr_url} target="_blank" rel="noreferrer">
                pull request{proposal.pr_number ? ` #${proposal.pr_number}` : ""}
              </a>
            </>
          )}{" "}
          <span className="muted">
            by {proposal.proposed_by}, {relative(proposal.state_since)}. The draft is read-only.
          </span>
        </p>
      )}
      {error && (
        <p className="warn" role="alert">
          {error}
        </p>
      )}

      {ops.length > 0 && (
        <ol className="plain" aria-label="Changes in the draft">
          {ops.map((o) => (
            <li key={o.seq} className={o.stale ? "lane-row stale" : "lane-row"} data-testid={`draft-op-${o.seq}`}>
              <span>
                {opSummary(o)}
                <div className="muted">
                  {o.author}, {relative(o.created_at)}
                </div>
                {o.stale && <div className="warn">no longer applies: {o.stale}</div>}
              </span>
              {!frozen && (
                <button type="button" aria-label={`Remove "${opSummary(o)}" from the draft`} disabled={busy} onClick={() => void run("The change was not removed", () => draftsApi.removeOp(o.seq))}>
                  Remove
                </button>
              )}
            </li>
          ))}
        </ol>
      )}
      {stale.length > 0 && !frozen && <p className="warn">Remove the {stale.length === 1 ? "change" : "changes"} that no longer {stale.length === 1 ? "applies" : "apply"} before proposing.</p>}

      <div className="draft-actions">
        {!frozen && (
          <button type="button" className="primary" disabled={busy || ops.length === 0 || stale.length > 0} onClick={() => void openReview()}>
            Propose map change…
          </button>
        )}
        {proposal && proposal.state !== "merged" && (
          <button type="button" disabled={busy} onClick={() => void withdraw()}>
            Withdraw proposal
          </button>
        )}
        {proposal?.state === "opening" && (
          <button type="button" className="primary" disabled={busy} onClick={() => void propose()}>
            Propose again
          </button>
        )}
        {!frozen && ops.length > 0 && (
          <button type="button" disabled={busy} onClick={() => void discard()}>
            Discard draft
          </button>
        )}
        {draft.can_restore && (
          <button type="button" disabled={busy} onClick={() => void restore()}>
            Undo discard
          </button>
        )}
        <button type="button" disabled={busy || ops.length === 0} onClick={() => void copyYaml()}>
          Copy YAML
        </button>
      </div>

      {review && (
        <div className="draft-review" role="group" aria-label="What the proposal changes in the map file">
          <h3>What changes in the map file</h3>
          <pre className="diff" data-testid="draft-diff">
            {review.diff.map((l, i) => (
              <span key={i} className={l.sign === "+" ? "add" : l.sign === "-" ? "del" : undefined}>
                {l.sign} {l.text}
                {"\n"}
              </span>
            ))}
          </pre>
          <p className="muted">Rubato opens one pull request on the map repository. The draft is read-only for everyone until it is merged, closed or withdrawn.</p>
          <div className="draft-actions">
            <button type="button" className="primary" disabled={busy} onClick={() => void propose()}>
              Open the pull request
            </button>
            <button type="button" disabled={busy} onClick={() => setReview(null)}>
              Not yet
            </button>
          </div>
        </div>
      )}
      {copied !== null && (
        <label className="stack">
          The proposed map file
          <textarea readOnly rows={8} value={copied} data-testid="draft-yaml" onFocus={(e) => e.currentTarget.select()} />
        </label>
      )}

      {!frozen && selectedNode && (
        <form className="stack draft-form" onSubmit={(e) => void updatePart(e)} key={`node-${selectedNode.id}-${ops.length}`} aria-label={`Change part ${selectedNode.id}`}>
          <h3>Change part {selectedNode.id}</h3>
          <label>
            Kind
            <select name="kind" defaultValue={selectedNode.kind}>
              {KINDS.map((k) => (
                <option key={k}>{k}</option>
              ))}
            </select>
          </label>
          <label>
            What it is
            <textarea name="description" rows={2} defaultValue={selectedNode.description ?? ""} />
          </label>
          <label>
            Repository (owner/name)
            <input name="repo" defaultValue={selectedNode.repo ?? ""} />
          </label>
          <label>
            Path in the repository
            <input name="path" defaultValue={selectedNode.path ?? ""} />
          </label>
          <div className="draft-actions">
            <button type="submit" disabled={busy}>
              Save part
            </button>
            <button type="button" disabled={busy} onClick={() => void post({ op: "remove_node", id: selectedNode.id }, `Part ${selectedNode.id} was not removed`)}>
              Remove part {selectedNode.id}
            </button>
          </div>
        </form>
      )}
      {!frozen && selectedSeam && (
        <form className="stack draft-form" onSubmit={(e) => void updateSeam(e)} key={`seam-${selectedSeam.id}-${ops.length}`} aria-label={`Change seam ${selectedSeam.id}`}>
          <h3>Change seam {selectedSeam.id}</h3>
          <p className="muted">
            {selectedSeam.from} depends on {selectedSeam.to}. The ends of a seam cannot be changed: remove it and add another.
          </p>
          <label>
            What it is for
            <textarea name="description" rows={2} defaultValue={selectedSeam.description ?? ""} />
          </label>
          <label>
            Owner (woken when it breaks)
            <select name="owner" defaultValue={selectedSeam.owner}>
              <option>{selectedSeam.from}</option>
              <option>{selectedSeam.to}</option>
            </select>
          </label>
          <label>
            Carries
            <input name="carries" defaultValue={selectedSeam.carries ?? ""} />
          </label>
          <label>
            Guard (the check that protects it)
            <input name="guard" defaultValue={selectedSeam.guard ?? ""} />
          </label>
          <label>
            Flows, one per line: <code>data: from -&gt; to: what it carries</code>
            <textarea name="flows" rows={4} defaultValue={flowsToText(selectedSeam.flows)} />
          </label>
          <div className="draft-actions">
            <button type="submit" disabled={busy}>
              Save seam
            </button>
            <button type="button" disabled={busy} onClick={() => void post({ op: "remove_seam", id: selectedSeam.id }, `Seam ${selectedSeam.id} was not removed`)}>
              Remove seam {selectedSeam.id}
            </button>
          </div>
        </form>
      )}

      {!frozen && (
        <>
          <form className="stack draft-form" onSubmit={(e) => void addPart(e)} aria-label="Add part">
            <h3>Add part</h3>
            <label>
              Id (lower-case letters, digits, hyphens; cannot be changed later)
              <input ref={addPartId} name="id" required pattern="[a-z0-9]+(-[a-z0-9]+)*" />
            </label>
            <label>
              Kind
              <select name="kind" defaultValue="context">
                {KINDS.map((k) => (
                  <option key={k}>{k}</option>
                ))}
              </select>
            </label>
            <label>
              What it is
              <input name="description" />
            </label>
            <label>
              Repository (owner/name)
              <input name="repo" />
            </label>
            <label>
              Path in the repository
              <input name="path" />
            </label>
            <button type="submit" disabled={busy}>
              Add part
            </button>
          </form>
          <form className="stack draft-form" onSubmit={(e) => void addSeam(e)} aria-label="Add seam">
            <h3>Add seam</h3>
            <label>
              From (the part that depends)
              <select name="from" required>
                {map.nodes.map((n) => (
                  <option key={n.id}>{n.id}</option>
                ))}
              </select>
            </label>
            <label>
              To (the part depended on)
              <select name="to" required defaultValue={map.nodes[1]?.id}>
                {map.nodes.map((n) => (
                  <option key={n.id}>{n.id}</option>
                ))}
              </select>
            </label>
            <label>
              Owner (woken when it breaks)
              <select name="owner" defaultValue="to">
                <option value="to">the To side</option>
                <option value="from">the From side</option>
              </select>
            </label>
            <label>
              Id (empty for from-to)
              <input name="id" pattern="[a-z0-9]+(-[a-z0-9]+)*" />
            </label>
            <label>
              Carries
              <input name="carries" />
            </label>
            <button type="submit" disabled={busy}>
              Add seam
            </button>
          </form>
        </>
      )}
    </section>
  );
}
