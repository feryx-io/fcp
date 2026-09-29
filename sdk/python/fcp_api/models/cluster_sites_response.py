from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.site_response import SiteResponse





T = TypeVar("T", bound="ClusterSitesResponse")



@_attrs_define
class ClusterSitesResponse:
    """ Response for GET /api/v1/cluster/sites: every tenant's sites, as the
    tenant API shows one.

        Attributes:
            available (bool):
            sites (list[SiteResponse]):
            message (None | str | Unset):
     """

    available: bool
    sites: list[SiteResponse]
    message: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.site_response import SiteResponse # noqa: PLC0415
        available = self.available

        sites = []
        for sites_item_data in self.sites:
            sites_item = sites_item_data.to_dict()
            sites.append(sites_item)



        message: None | str | Unset
        if isinstance(self.message, Unset):
            message = UNSET
        else:
            message = self.message


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "available": available,
            "sites": sites,
        })
        if message is not UNSET:
            field_dict["message"] = message

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.site_response import SiteResponse # noqa: PLC0415
        d = dict(src_dict)
        available = d.pop("available")

        sites = []
        _sites = d.pop("sites")
        for sites_item_data in (_sites):
            sites_item = SiteResponse.from_dict(sites_item_data)



            sites.append(sites_item)


        def _parse_message(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        message = _parse_message(d.pop("message", UNSET))


        cluster_sites_response = cls(
            available=available,
            sites=sites,
            message=message,
        )


        cluster_sites_response.additional_properties = d
        return cluster_sites_response

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
