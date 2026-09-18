import { test } from "node:test";
import assert from "node:assert/strict";
import { EditSession, jobCompatible } from "../src/lib/runtime.ts";
import { createProject, updateDocument } from "../src/lib/project.ts";

test("edit queue snapshots proposals and uses acknowledged revisions in order", async () => {
  const base = createProject();
  const calls = [];
  let release;
  const runtime = new EditSession(async (command, args) => {
    if (command === "open_edit_session") return { sessionId: "session", project: base };
    calls.push(args);
    if (calls.length === 1) await new Promise(resolve => { release = resolve; });
    return { ...args.proposed, revision: args.expectedRevision + 10 };
  });
  await runtime.open(base);
  const proposed = { ...base, name: "First", revision: 999 };
  const first = runtime.edit(proposed);
  proposed.name = "Mutated after enqueue";
  const second = runtime.edit({ ...base, name: "Second" }, true);
  let flushed = false;
  const flushing = runtime.flush().then(project => { flushed = true; return project; });
  await Promise.resolve();
  assert.equal(calls.length, 1);
  assert.equal(flushed, false);
  release();
  await Promise.all([first, second]);
  assert.equal((await flushing).revision, 20);
  assert.equal(calls[0].proposed.name, "First");
  assert.deepEqual(calls.map(c => c.expectedRevision), [0, 10]);
  assert.equal(calls[1].invalidateDocuments, true);
  assert.equal(calls[1].sessionId, "session");
});

test("failed edit stops queued snapshots and reopening recovers the session", async () => {
  const base = createProject();
  let edits = 0;
  let fail = true;
  const runtime = new EditSession(async (command, args) => {
    if (command === "open_edit_session") return { sessionId: "session", project: args.project };
    edits++;
    if (fail) throw new Error("Disk failure");
    return { ...args.proposed, revision: args.expectedRevision + 1 };
  });
  await runtime.open(base);
  const results = await Promise.allSettled([runtime.edit({ ...base, name: "A" }), runtime.edit({ ...base, name: "B" })]);
  assert.deepEqual(results.map(r => r.status), ["rejected", "rejected"]);
  assert.equal(edits, 1);
  await assert.rejects(runtime.flush(), /Disk failure/);
  fail = false;
  await runtime.open({ ...base, name: "Visible work" });
  assert.equal(runtime.error, null);
  assert.equal((await runtime.edit({ ...base, name: "Recovered" })).revision, 1);
});

test("job acceptance flushes pending edits and consumes server revision", async () => {
  const base = createProject();
  const commands = [];
  const runtime = new EditSession(async (command, args) => {
    commands.push({ command, args });
    if (command === "open_edit_session") return { sessionId: "session", project: base };
    if (command === "apply_project_edit") return { ...args.proposed, revision: 7 };
    return { ...base, revision: 8 };
  });
  await runtime.open(base);
  const editing = runtime.edit({ ...base, name: "Changed" });
  const result = await runtime.resolveJob("accept_generation_job", "job");
  await editing;
  assert.equal(commands[2].args.expectedRevision, 7);
  assert.equal(commands[2].args.jobId, "job");
  assert.equal(result.revision, 8);
  assert.equal((await runtime.flush()).revision, 8);
});

test("job compatibility checks complete document content, not only revisions", () => {
  const base = createProject();
  const project = updateDocument(base, base.activeDocumentId, { sections: [{ title: "Model", content: "Original", diagrams: [
    { content: "class A", diagram_type: "class", format: "plantuml" },
  ] }] });
  const job = { projectId: project.id, documentId: project.activeDocumentId, status: "ready", baseDocument: structuredClone(project.documents[0]) };
  assert.equal(jobCompatible(project, job), true);
  assert.equal(jobCompatible({ ...project, name: "Unrelated project rename" }, job), true);
  for (const mutate of [
    d => { d.name = "Different"; }, d => { d.template = "c4"; }, d => { d.language = "sk"; },
    d => { d.sections[0].content = "Other branch, equal revision"; },
    d => { d.sections[0].diagrams[0].content = "class B"; },
    d => { d.revision++; },
  ]) {
    const changed = structuredClone(project); mutate(changed.documents[0]);
    assert.equal(jobCompatible(changed, job), false);
  }
  assert.equal(jobCompatible({ ...project, id: "other" }, job), false);
  assert.equal(jobCompatible({ ...project, documents: [] }, job), false);
  assert.equal(jobCompatible(project, { ...job, status: "cancelled" }), false);
});
