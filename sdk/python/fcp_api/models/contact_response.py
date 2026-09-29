from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ContactResponse")



@_attrs_define
class ContactResponse:
    """ One notification contact as the API shows it.

        Attributes:
            kind (str): `email`, `sms`, `call` or `webhook`.
            target (str):
            name (None | str | Unset):
            issues (list[str] | Unset): What keeps this contact from being delivered to, empty when it can.
     """

    kind: str
    target: str
    name: None | str | Unset = UNSET
    issues: list[str] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        kind = self.kind

        target = self.target

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        issues: list[str] | Unset = UNSET
        if not isinstance(self.issues, Unset):
            issues = self.issues




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "kind": kind,
            "target": target,
        })
        if name is not UNSET:
            field_dict["name"] = name
        if issues is not UNSET:
            field_dict["issues"] = issues

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        kind = d.pop("kind")

        target = d.pop("target")

        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        issues = cast(list[str], d.pop("issues", UNSET))


        contact_response = cls(
            kind=kind,
            target=target,
            name=name,
            issues=issues,
        )


        contact_response.additional_properties = d
        return contact_response

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
