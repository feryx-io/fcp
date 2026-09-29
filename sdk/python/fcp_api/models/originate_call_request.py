from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="OriginateCallRequest")



@_attrs_define
class OriginateCallRequest:
    """ 
        Attributes:
            from_ (str): Extension or SIP URI of the party to ring first (caller).
            to (str): Extension or SIP URI of the destination party (callee).
            from_display_name (None | str | Unset): Optional display name for the caller.
            tenant_id (None | str | Unset): Optional tenant ID (uses server default if omitted).
     """

    from_: str
    to: str
    from_display_name: None | str | Unset = UNSET
    tenant_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from_ = self.from_

        to = self.to

        from_display_name: None | str | Unset
        if isinstance(self.from_display_name, Unset):
            from_display_name = UNSET
        else:
            from_display_name = self.from_display_name

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "from": from_,
            "to": to,
        })
        if from_display_name is not UNSET:
            field_dict["from_display_name"] = from_display_name
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        from_ = d.pop("from")

        to = d.pop("to")

        def _parse_from_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        from_display_name = _parse_from_display_name(d.pop("from_display_name", UNSET))


        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        originate_call_request = cls(
            from_=from_,
            to=to,
            from_display_name=from_display_name,
            tenant_id=tenant_id,
        )


        originate_call_request.additional_properties = d
        return originate_call_request

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
