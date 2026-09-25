import { StrKey } from "@stellar/stellar-sdk";
import { z } from "zod";

/** Zod validator for a Stellar account (G...) or contract (C...) StrKey. */
export const stellarAddressSchema = z.string().refine(
  (address) =>
    StrKey.isValidEd25519PublicKey(address) || StrKey.isValidContract(address),
  { message: "must be a valid Stellar address" },
);
