from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="ContactReadiness")



@_attrs_define
class ContactReadiness:
    """ One contact and how it has fared (§8.4): a tenant should be able to
    see that its notifications work before it needs them to.

        Attributes:
            kind (str):
            target (str):
            issues (list[str]): What keeps this contact from being delivered to.
            name (None | str | Unset):
            last_delivery_at (datetime.datetime | None | Unset): When anything was last delivered to it.
            last_test_at (datetime.datetime | None | Unset): When a `933` test call's notice last reached it.
     """

    kind: str
    target: str
    issues: list[str]
    name: None | str | Unset = UNSET
    last_delivery_at: datetime.datetime | None | Unset = UNSET
    last_test_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        kind = self.kind

        target = self.target

        issues = self.issues



        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        last_delivery_at: None | str | Unset
        if isinstance(self.last_delivery_at, Unset):
            last_delivery_at = UNSET
        elif isinstance(self.last_delivery_at, datetime.datetime):
            last_delivery_at = self.last_delivery_at.isoformat()
        else:
            last_delivery_at = self.last_delivery_at

        last_test_at: None | str | Unset
        if isinstance(self.last_test_at, Unset):
            last_test_at = UNSET
        elif isinstance(self.last_test_at, datetime.datetime):
            last_test_at = self.last_test_at.isoformat()
        else:
            last_test_at = self.last_test_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "kind": kind,
            "target": target,
            "issues": issues,
        })
        if name is not UNSET:
            field_dict["name"] = name
        if last_delivery_at is not UNSET:
            field_dict["last_delivery_at"] = last_delivery_at
        if last_test_at is not UNSET:
            field_dict["last_test_at"] = last_test_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        kind = d.pop("kind")

        target = d.pop("target")

        issues = cast(list[str], d.pop("issues"))


        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_last_delivery_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_delivery_at_type_0 = datetime.datetime.fromisoformat(data)



                return last_delivery_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_delivery_at = _parse_last_delivery_at(d.pop("last_delivery_at", UNSET))


        def _parse_last_test_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_test_at_type_0 = datetime.datetime.fromisoformat(data)



                return last_test_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_test_at = _parse_last_test_at(d.pop("last_test_at", UNSET))


        contact_readiness = cls(
            kind=kind,
            target=target,
            issues=issues,
            name=name,
            last_delivery_at=last_delivery_at,
            last_test_at=last_test_at,
        )


        contact_readiness.additional_properties = d
        return contact_readiness

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
