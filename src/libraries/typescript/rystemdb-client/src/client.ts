/** RED test scaffold for localhost-only Node TCP RSWP client. */
export class RystemLocalClient {
  static async connect(_host: string, _port: number): Promise<RystemLocalClient> {
    throw new Error("TCP client not implemented");
  }
  async ping(): Promise<void> { throw new Error("PING not implemented"); }
  async put(_collection: string, _id: string, _document: unknown): Promise<void> {
    throw new Error("PUT not implemented");
  }
  async get(_collection: string, _id: string): Promise<unknown | null> {
    throw new Error("GET not implemented");
  }
  async delete(_collection: string, _id: string): Promise<boolean> {
    throw new Error("DELETE not implemented");
  }
  async close(): Promise<void> {}
}
