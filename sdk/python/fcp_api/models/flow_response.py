from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="FlowResponse")



@_attrs_define
class FlowResponse:
    """ Flow response model.

        Attributes:
            id (str):
            name (str):
            version (int):
            initial_node (str):
            nodes (Any):
            variables (Any):
            enabled (bool):
            description (None | str | Unset):
            tenant_id (None | str | Unset):
            webhook_url (None | str | Unset):
            created_at (None | str | Unset):
            updated_at (None | str | Unset):
     """

    id: str
    name: str
    version: int
    initial_node: str
    nodes: Any
    variables: Any
    enabled: bool
    description: None | str | Unset = UNSET
    tenant_id: None | str | Unset = UNSET
    webhook_url: None | str | Unset = UNSET
    created_at: None | str | Unset = UNSET
    updated_at: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        name = self.name

        version = self.version

        initial_node = self.initial_node

        nodes = self.nodes

        variables = self.variables

        enabled = self.enabled

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        webhook_url: None | str | Unset
        if isinstance(self.webhook_url, Unset):
            webhook_url = UNSET
        else:
            webhook_url = self.webhook_url

        created_at: None | str | Unset
        if isinstance(self.created_at, Unset):
            created_at = UNSET
        else:
            created_at = self.created_at

        updated_at: None | str | Unset
        if isinstance(self.updated_at, Unset):
            updated_at = UNSET
        else:
            updated_at = self.updated_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "version": version,
            "initial_node": initial_node,
            "nodes": nodes,
            "variables": variables,
            "enabled": enabled,
        })
        if description is not UNSET:
            field_dict["description"] = description
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if webhook_url is not UNSET:
            field_dict["webhook_url"] = webhook_url
        if created_at is not UNSET:
            field_dict["created_at"] = created_at
        if updated_at is not UNSET:
            field_dict["updated_at"] = updated_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        version = d.pop("version")

        initial_node = d.pop("initial_node")

        nodes = d.pop("nodes")

        variables = d.pop("variables")

        enabled = d.pop("enabled")

        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        def _parse_webhook_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        webhook_url = _parse_webhook_url(d.pop("webhook_url", UNSET))


        def _parse_created_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        created_at = _parse_created_at(d.pop("created_at", UNSET))


        def _parse_updated_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        updated_at = _parse_updated_at(d.pop("updated_at", UNSET))


        flow_response = cls(
            id=id,
            name=name,
            version=version,
            initial_node=initial_node,
            nodes=nodes,
            variables=variables,
            enabled=enabled,
            description=description,
            tenant_id=tenant_id,
            webhook_url=webhook_url,
            created_at=created_at,
            updated_at=updated_at,
        )


        flow_response.additional_properties = d
        return flow_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
