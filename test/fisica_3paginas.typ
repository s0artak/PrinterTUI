#set page(
  paper: "a4",
  margin: (x: 2cm, y: 2.5cm),
  header: none,
  footer: none
)

#set text(font: "Liberation Serif", size: 28pt)
#let hbar = $ℏ$

// PÁGINA 1: Ecuaciones de Maxwell (forma diferencial más reducida)
#align(center + horizon)[
  $
  dif F = 0 \
  \
  dif star F = mu_0 J
  $
]

#pagebreak()

// PÁGINA 2: Ecuación de Schrödinger
#align(center + horizon)[
  $
  i hbar frac(partial, partial t) |psi(t) chevron.r = hat(H) |psi(t) chevron.r
  $
]

#pagebreak()

// PÁGINA 3: Lagrangiano de mecánica cuántica (sistema de partículas)
#align(center + horizon)[
  #set text(size: 22pt)
  $
  L = sum_(k=1)^N 1/2 m_k dot(bold(r))_k^2 - sum_(k=1)^N V(bold(r)_k) - sum_(1 <= i < j <= N) U(bold(r)_i - bold(r)_j) \
  \
  \
  K(b, a) = integral cal(D)[bold(R)(t)] exp( i/hbar integral_(t_a)^(t_b) L dif t )
  $
]
