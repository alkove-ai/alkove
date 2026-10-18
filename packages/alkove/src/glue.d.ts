declare module 'alkove:ort-glue' {
  import type { OrtFactory } from './engine/ort'
  const factory: OrtFactory
  export default factory
}
