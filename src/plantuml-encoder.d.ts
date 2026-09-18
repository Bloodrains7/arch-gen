declare module "plantuml-encoder" {
  const encoder: { encode(content: string): string };
  export default encoder;
}
