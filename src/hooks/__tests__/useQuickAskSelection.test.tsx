import { act, renderHook } from "@testing-library/react";
import { api } from "@/lib/tauri";
import { useMailStore } from "@/stores/mailStore";
import { useQuickAskSelection } from "../useQuickAskSelection";

beforeEach(() => {
  vi.useFakeTimers();
  useMailStore.setState({
    selectedAccountId: "view-account", selectedFolder: "INBOX",
    selectedMessageAccountId: "message-account", selectedMessageFolder: "Archive",
    selectedMessageUid: 42, selectedMessageUids: new Set(),
    isUnifiedInbox: true, selectedGroupId: null, specialView: null, isComposing: false,
  });
});
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); });

it("publishes the selected message's own account and folder without sending mail text", async () => {
  const publish = vi.spyOn(api.quickAsk, "publishSelection").mockResolvedValue(undefined);
  renderHook(useQuickAskSelection);
  await act(async () => {});
  expect(publish).toHaveBeenCalledExactlyOnceWith({
    account_id: "message-account", folder: "Archive", uid: 42, composing: false, view: "INBOX",
  });
  act(() => useMailStore.setState({ selectedMessageUids: new Set(["a|INBOX|1", "b|INBOX|2"]) }));
  await act(async () => {});
  expect(publish).toHaveBeenLastCalledWith(expect.objectContaining({ uid: null }));
});

it("serializes publication so an earlier slow call cannot replace the latest selection", async () => {
  let finish!: () => void;
  const publish = vi.spyOn(api.quickAsk, "publishSelection")
    .mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }))
    .mockResolvedValue(undefined);
  renderHook(useQuickAskSelection);
  await act(async () => {});
  act(() => useMailStore.setState({ selectedMessageUid: 43 }));
  await act(async () => {});
  expect(publish).toHaveBeenCalledTimes(1);
  await act(async () => finish());
  expect(publish).toHaveBeenCalledTimes(2);
  expect(publish).toHaveBeenLastCalledWith(expect.objectContaining({ uid: 43 }));
});

it("retries an unavailable bridge and stops publishing after unmount", async () => {
  const publish = vi.spyOn(api.quickAsk, "publishSelection")
    .mockRejectedValueOnce(new Error("Starting"))
    .mockResolvedValue(undefined);
  const { unmount } = renderHook(useQuickAskSelection);
  await act(async () => {});
  await act(async () => vi.advanceTimersByTimeAsync(2000));
  expect(publish).toHaveBeenCalledTimes(2);
  unmount();
  act(() => useMailStore.setState({ selectedMessageUid: 99 }));
  await act(async () => vi.advanceTimersByTimeAsync(4000));
  expect(publish).toHaveBeenCalledTimes(2);
});
