/** A stand-in for `DataTransfer`, which jsdom does not implement. Pass it as `{ dataTransfer }` to `fireEvent.drag*`. */
export function dataTransfer(): DataTransfer {
  const store = new Map<string, string>();
  const dt = {
    dropEffect: "none",
    effectAllowed: "uninitialized",
    get types() {
      return [...store.keys()];
    },
    setData: (type: string, value: string) => void store.set(type, value),
    getData: (type: string) => store.get(type) ?? "",
  };
  return dt as unknown as DataTransfer;
}
