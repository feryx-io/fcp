import { useState, useCallback, useEffect } from 'react'
import { useParams, useNavigate } from 'react-router-dom'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  ReactFlow,
  Controls,
  Background,
  addEdge,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
  BackgroundVariant,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  Save, ArrowLeft, Trash2, Play, Mic, Phone,
  Voicemail, GitBranch, PhoneOff, Globe, Users,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Badge } from '@/components/ui/badge'
// Card imports reserved for future property panel enhancements
import { flows } from '@/api/endpoints'
import type { FlowNode } from '@/types'

const NODE_TYPES_PALETTE = [
  { type: 'say', label: 'Say', icon: Mic, color: '#3b82f6' },
  { type: 'play', label: 'Play', icon: Play, color: '#8b5cf6' },
  { type: 'gather', label: 'Gather', icon: Phone, color: '#10b981' },
  { type: 'dial', label: 'Dial', icon: Phone, color: '#f59e0b' },
  { type: 'queue', label: 'Queue', icon: Users, color: '#6366f1' },
  { type: 'voicemail', label: 'Voicemail', icon: Voicemail, color: '#ec4899' },
  { type: 'branch', label: 'Branch', icon: GitBranch, color: '#14b8a6' },
  { type: 'http', label: 'HTTP', icon: Globe, color: '#f97316' },
  { type: 'hangup', label: 'Hangup', icon: PhoneOff, color: '#ef4444' },
]

function flowNodesToReactFlow(flowNodes: Record<string, FlowNode>, initialNode: string): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = []
  const edges: Edge[] = []
  const entries = Object.entries(flowNodes)

  entries.forEach(([id, node], index) => {
    const palette = NODE_TYPES_PALETTE.find((p) => p.type === node.type)
    nodes.push({
      id,
      type: 'default',
      position: { x: 250, y: index * 120 },
      data: {
        label: (
          <div className="text-xs">
            <div className="font-bold" style={{ color: palette?.color }}>{node.label || id}</div>
            <div className="text-muted-foreground">{node.type}</div>
          </div>
        ),
      },
      style: {
        border: `2px solid ${palette?.color ?? '#888'}`,
        borderRadius: 8,
        padding: 8,
        background: 'var(--card)',
      },
    })

    // Create edges from "next" field
    const next = (node as Record<string, unknown>).next as string | null
    if (next && flowNodes[next]) {
      edges.push({
        id: `${id}->${next}`,
        source: id,
        target: next,
        markerEnd: { type: MarkerType.ArrowClosed },
        animated: id === initialNode,
      })
    }

    // Branch edges
    if (node.type === 'gather' || node.type === 'branch') {
      const branches = (node as Record<string, unknown>).branches as Record<string, string> | undefined
      if (branches) {
        Object.entries(branches).forEach(([key, target]) => {
          if (flowNodes[target]) {
            edges.push({
              id: `${id}->${target}[${key}]`,
              source: id,
              target,
              label: key,
              markerEnd: { type: MarkerType.ArrowClosed },
            })
          }
        })
      }
    }
  })

  return { nodes, edges }
}

function reactFlowToFlowNodes(
  nodes: Node[],
  edges: Edge[],
  existingFlowNodes: Record<string, FlowNode>,
): Record<string, FlowNode> {
  const result: Record<string, FlowNode> = {}

  nodes.forEach((node) => {
    const existing = existingFlowNodes[node.id] || { type: 'say' }
    const outgoing = edges.filter((e) => e.source === node.id)
    const mainEdge = outgoing.find((e) => !e.label)

    result[node.id] = {
      ...existing,
      next: mainEdge?.target ?? null,
    } as FlowNode & { next: string | null }
  })

  return result
}

export function FlowBuilderPage() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const { data: flow, isLoading } = useQuery({
    queryKey: ['flow', id],
    queryFn: () => flows.get(id!),
    enabled: !!id,
  })

  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([])
  const [selectedNode, setSelectedNode] = useState<string | null>(null)
  const [flowName, setFlowName] = useState('')
  const [nodeCounter, setNodeCounter] = useState(0)

  // Initialize from flow data
  useEffect(() => {
    if (flow) {
      const { nodes: rfNodes, edges: rfEdges } = flowNodesToReactFlow(
        flow.nodes || {},
        flow.initial_node,
      )
      setNodes(rfNodes)
      setEdges(rfEdges)
      setFlowName(flow.name)
      setNodeCounter(Object.keys(flow.nodes || {}).length)
    }
  }, [flow, setNodes, setEdges])

  const onConnect = useCallback((connection: Connection) => {
    setEdges((eds) => addEdge({
      ...connection,
      markerEnd: { type: MarkerType.ArrowClosed },
    }, eds))
  }, [setEdges])

  const saveMutation = useMutation({
    mutationFn: () => {
      const flowNodes = reactFlowToFlowNodes(nodes, edges, flow?.nodes || {})
      return flows.update(id!, {
        name: flowName,
        nodes: flowNodes,
      })
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['flow', id] })
      queryClient.invalidateQueries({ queryKey: ['flows'] })
    },
  })

  const addNode = (type: string) => {
    const palette = NODE_TYPES_PALETTE.find((p) => p.type === type)
    const newId = `${type}_${nodeCounter}`
    setNodeCounter((c) => c + 1)

    const newNode: Node = {
      id: newId,
      type: 'default',
      position: { x: 300 + Math.random() * 100, y: 100 + nodes.length * 80 },
      data: {
        label: (
          <div className="text-xs">
            <div className="font-bold" style={{ color: palette?.color }}>{newId}</div>
            <div className="text-muted-foreground">{type}</div>
          </div>
        ),
      },
      style: {
        border: `2px solid ${palette?.color ?? '#888'}`,
        borderRadius: 8,
        padding: 8,
        background: 'var(--card)',
      },
    }
    setNodes((nds) => [...nds, newNode])
  }

  const deleteSelectedNode = () => {
    if (!selectedNode) return
    setNodes((nds) => nds.filter((n) => n.id !== selectedNode))
    setEdges((eds) => eds.filter((e) => e.source !== selectedNode && e.target !== selectedNode))
    setSelectedNode(null)
  }

  if (isLoading) {
    return <div className="flex h-full items-center justify-center text-muted-foreground">Loading flow...</div>
  }

  return (
    <div className="flex flex-col h-[calc(100vh-7rem)]">
      {/* Toolbar */}
      <div className="flex items-center justify-between border-b bg-card px-4 py-2">
        <div className="flex items-center gap-3">
          <Button variant="ghost" size="icon" onClick={() => navigate('/admin/flows')}>
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <Input
            value={flowName}
            onChange={(e) => setFlowName(e.target.value)}
            className="w-48 h-8 text-sm font-medium"
          />
          <Badge variant="outline">v{flow?.version ?? 1}</Badge>
          <Badge variant={flow?.enabled ? 'success' : 'secondary'}>
            {flow?.enabled ? 'Active' : 'Disabled'}
          </Badge>
        </div>
        <div className="flex items-center gap-2">
          {selectedNode && (
            <Button variant="outline" size="sm" onClick={deleteSelectedNode}>
              <Trash2 className="mr-1 h-3 w-3" />
              Delete Node
            </Button>
          )}
          <Button
            size="sm"
            onClick={() => saveMutation.mutate()}
            disabled={saveMutation.isPending}
          >
            <Save className="mr-1 h-3 w-3" />
            {saveMutation.isPending ? 'Saving...' : 'Save'}
          </Button>
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">
        {/* Node Palette */}
        <div className="w-48 border-r bg-card p-3 overflow-y-auto">
          <p className="text-xs font-semibold text-muted-foreground mb-2 uppercase">Add Node</p>
          <div className="space-y-1">
            {NODE_TYPES_PALETTE.map((item) => {
              const Icon = item.icon
              return (
                <Button
                  key={item.type}
                  variant="ghost"
                  size="sm"
                  className="w-full justify-start gap-2 text-xs"
                  onClick={() => addNode(item.type)}
                >
                  <Icon className="h-3 w-3" style={{ color: item.color }} />
                  {item.label}
                </Button>
              )
            })}
          </div>
        </div>

        {/* React Flow Canvas */}
        <div className="flex-1">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={(_, node) => setSelectedNode(node.id)}
            onPaneClick={() => setSelectedNode(null)}
            fitView
          >
            <Controls />
            <Background variant={BackgroundVariant.Dots} gap={16} />
          </ReactFlow>
        </div>

        {/* Properties Panel */}
        {selectedNode && flow?.nodes?.[selectedNode] && (
          <div className="w-64 border-l bg-card p-4 overflow-y-auto">
            <h3 className="font-semibold text-sm mb-3">Node Properties</h3>
            <div className="space-y-3 text-sm">
              <div>
                <Label className="text-xs">ID</Label>
                <p className="font-mono text-xs bg-muted rounded px-2 py-1">{selectedNode}</p>
              </div>
              <div>
                <Label className="text-xs">Type</Label>
                <p className="capitalize">{flow.nodes[selectedNode].type}</p>
              </div>
              {Object.entries(flow.nodes[selectedNode])
                .filter(([k]) => k !== 'type' && k !== 'label')
                .map(([key, value]) => (
                  <div key={key}>
                    <Label className="text-xs">{key}</Label>
                    <p className="text-xs font-mono bg-muted rounded px-2 py-1 break-all">
                      {typeof value === 'object' ? JSON.stringify(value) : String(value ?? '-')}
                    </p>
                  </div>
                ))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
