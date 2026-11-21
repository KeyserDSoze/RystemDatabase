/** Experimental CRUD key codec contract; RED until implemented. */
export interface CrudKey {
    collection: string;
    id: string;
    document: Uint8Array;
}
export function encodeCrudKey(_collection: string, _id: string): Uint8Array {
    throw new Error("CRUD key codec missing");
}
export function decodeCrudKey(_bytes: Uint8Array): CrudKey {
    throw new Error("CRUD key codec missing");
}
